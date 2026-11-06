// Raw RandomX throughput probe: NUMA-local datasets + one thread per physical
// core + 2 MiB-aligned transparent-huge-page allocations (patched RandomX).
#include <randomx.h>

#include <atomic>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <map>
#include <set>
#include <sstream>
#include <string>
#include <thread>
#include <vector>

#include <sched.h>
#include <sys/syscall.h>
#include <unistd.h>

#ifndef MPOL_BIND
#define MPOL_BIND 2
#endif

namespace {

struct Core {
    int cpu = 0;
    int node = 0;
    int package = 0;
    int core = 0;
};

int readInt(const std::string &path) {
    std::ifstream f(path);
    int v = -1;
    f >> v;
    return v;
}

std::vector<int> parseCpuList(const std::string &s) {
    std::vector<int> out;
    std::stringstream ss(s);
    std::string tok;
    while (std::getline(ss, tok, ',')) {
        if (tok.empty()) continue;
        auto dash = tok.find('-');
        if (dash == std::string::npos) {
            out.push_back(std::stoi(tok));
        } else {
            int a = std::stoi(tok.substr(0, dash));
            int b = std::stoi(tok.substr(dash + 1));
            for (int i = a; i <= b; ++i) out.push_back(i);
        }
    }
    return out;
}

int cpuNode(int cpu) {
    char path[256];
    snprintf(path, sizeof(path), "/sys/devices/system/cpu/cpu%d", cpu);
    for (int n = 0; n < 64; ++n) {
        char p[320];
        snprintf(p, sizeof(p), "%s/node%d", path, n);
        if (access(p, F_OK) == 0) return n;
    }
    return 0;
}

bool primaryThread(int cpu) {
    char path[256];
    snprintf(path, sizeof(path), "/sys/devices/system/cpu/cpu%d/topology/thread_siblings_list", cpu);
    std::ifstream f(path);
    std::string s;
    if (!std::getline(f, s)) return false;
    auto cpus = parseCpuList(s);
    return !cpus.empty() && cpus.front() == cpu;
}

std::vector<Core> topology() {
    std::vector<Core> cores;
    long n = sysconf(_SC_NPROCESSORS_ONLN);
    for (int cpu = 0; cpu < n; ++cpu) {
        if (!primaryThread(cpu)) continue;
        Core c;
        c.cpu = cpu;
        c.node = cpuNode(cpu);
        char path[256];
        snprintf(path, sizeof(path), "/sys/devices/system/cpu/cpu%d/topology/physical_package_id", cpu);
        c.package = readInt(path);
        snprintf(path, sizeof(path), "/sys/devices/system/cpu/cpu%d/topology/core_id", cpu);
        c.core = readInt(path);
        cores.push_back(c);
    }
    return cores;
}

bool pinToCpu(int cpu) {
    cpu_set_t set;
    CPU_ZERO(&set);
    CPU_SET(cpu, &set);
    return sched_setaffinity(0, sizeof(set), &set) == 0;
}

bool bindMemoryNode(int node) {
    unsigned long mask[16] = {0};
    if (node < 0 || node >= 1024) return false;
    mask[node / 64] |= 1UL << (node % 64);
    long rc = syscall(SYS_set_mempolicy, MPOL_BIND, mask, (unsigned long) 1024);
    return rc == 0;
}

uint64_t anonHugeKb() {
    std::ifstream f("/proc/self/smaps_rollup");
    std::string line;
    while (std::getline(f, line)) {
        if (line.rfind("AnonHugePages:", 0) == 0) {
            return std::strtoull(line.c_str() + 14, nullptr, 10);
        }
    }
    return 0;
}

struct Shared {
    std::vector<std::thread> workers;
    std::atomic<bool> start{false};
    std::atomic<bool> stop{false};
    std::atomic<uint64_t> hashes{0};
    std::atomic<uint64_t> errors{0};
};

} // namespace

int main(int argc, char **argv) {
    int seconds = argc > 1 ? std::atoi(argv[1]) : 20;
    bool useSmt = argc > 2 && std::strcmp(argv[2], "smt") == 0;

    auto cores = topology();
    std::map<int, std::vector<Core>> byNode;
    for (const auto &c : cores) byNode[c.node].push_back(c);

    std::printf("physical cores: %zu, NUMA nodes: %zu\n", cores.size(), byNode.size());
    for (auto &[node, list] : byNode)
        std::printf("  node %d: %zu physical cores\n", node, list.size());

    // Build one dataset and cache per NUMA node, allocated and first-touched
    // while the allocating thread is bound to that node.
    struct NodeData {
        randomx_cache *cache = nullptr;
        randomx_dataset *dataset = nullptr;
        std::vector<Core> cores;
    };
    std::map<int, NodeData> nd;
    const char seed[32] = "RxMiner-NUMA-probe-seed-0000001";
    std::vector<std::thread> initThreads;

    for (auto &[node, list] : byNode) {
        auto &d = nd[node];
        d.cores = list;
        initThreads.emplace_back([&, node] {
            pinToCpu(list.front().cpu);
            bindMemoryNode(node);
            d.cache = randomx_alloc_cache(RANDOMX_FLAG_DEFAULT);
            if (!d.cache) { std::fprintf(stderr, "cache alloc failed node %d\n", node); std::abort(); }
            randomx_init_cache(d.cache, seed, sizeof(seed));
            d.dataset = randomx_alloc_dataset(RANDOMX_FLAG_DEFAULT);
            if (!d.dataset) { std::fprintf(stderr, "dataset alloc failed node %d\n", node); std::abort(); }
            std::printf("node %d: cache+dataset allocated, AnonHugePages=%llu MiB\n", node,
                        (unsigned long long) (anonHugeKb() / 1024));
        });
    }
    for (auto &t : initThreads) t.join();

    auto t0 = std::chrono::steady_clock::now();
    for (auto &[node, d] : nd) {
        uint64_t items = randomx_dataset_item_count();
        unsigned nthreads = (unsigned) d.cores.size();
        std::vector<std::thread> ts;
        for (unsigned i = 0; i < nthreads; ++i) {
            ts.emplace_back([&, i, nthreads, node] {
                auto &dd = nd[node];
                pinToCpu(dd.cores[i].cpu);
                bindMemoryNode(node);
                uint64_t begin = items * i / nthreads;
                uint64_t end = items * (i + 1) / nthreads;
                randomx_init_dataset(dd.dataset, dd.cache, begin, end - begin);
            });
        }
        for (auto &t : ts) t.join();
    }
    auto t1 = std::chrono::steady_clock::now();
    std::printf("datasets ready in %lld ms, AnonHugePages=%llu MiB\n",
                (long long) std::chrono::duration_cast<std::chrono::milliseconds>(t1 - t0).count(),
                (unsigned long long) (anonHugeKb() / 1024));

    Shared sh;
    std::vector<std::thread> workers;
    const randomx_flags flags = (randomx_flags) (RANDOMX_FLAG_JIT | RANDOMX_FLAG_HARD_AES | RANDOMX_FLAG_FULL_MEM);

    for (auto &[node, d] : nd) {
        for (auto c : d.cores) {
            workers.emplace_back([&, node, c] {
                pinToCpu(c.cpu);
                bindMemoryNode(node);
                randomx_vm *vm = randomx_create_vm(flags, nullptr, nd[node].dataset);
                if (!vm) { sh.errors++; return; }
                uint8_t input[76];
                std::memset(input, 0x42, sizeof(input));
                uint64_t local = 0;
                while (!sh.start.load(std::memory_order_acquire)) {}
                while (!sh.stop.load(std::memory_order_relaxed)) {
                    uint64_t n = local;
                    std::memcpy(input + 64, &n, sizeof(n));
                    uint8_t hash[32];
                    randomx_calculate_hash(vm, input, sizeof(input), hash);
                    ++local;
                }
                sh.hashes.fetch_add(local, std::memory_order_relaxed);
                randomx_destroy_vm(vm);
            });
        }
    }

    std::this_thread::sleep_for(std::chrono::milliseconds(200));
    sh.start.store(true, std::memory_order_release);
    auto start = std::chrono::steady_clock::now();
    std::this_thread::sleep_for(std::chrono::seconds(seconds));
    sh.stop.store(true, std::memory_order_release);
    auto end = std::chrono::steady_clock::now();
    for (auto &t : workers) t.join();

    double secs = std::chrono::duration<double>(end - start).count();
    uint64_t total = sh.hashes.load();
    std::printf("threads=%zu seconds=%.2f hashes=%llu speed=%.1f H/s (%.1f H/s/thread) errors=%llu\n",
                workers.size(), secs, (unsigned long long) total, total / secs,
                total / secs / (double) workers.size(), (unsigned long long) sh.errors.load());
    std::printf("final AnonHugePages=%llu MiB\n", (unsigned long long) (anonHugeKb() / 1024));

    for (auto &[node, d] : nd) {
        randomx_release_dataset(d.dataset);
        randomx_release_cache(d.cache);
    }
    return 0;
}
