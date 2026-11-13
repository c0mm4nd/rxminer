// SPDX-License-Identifier: BSD-3-Clause
//! Safe ownership interface for heap or platform-provided initialized buffers.
use std::ops::{Deref, DerefMut};

pub trait Buffer<T>: AsRef<[T]> + AsMut<[T]> + Send + Sync {
    fn page_size(&self) -> usize {
        0
    }
}

pub enum Memory<T> {
    Heap(Vec<T>),
    Platform(Box<dyn Buffer<T>>),
}
impl<T> From<Vec<T>> for Memory<T> {
    fn from(value: Vec<T>) -> Self {
        Self::Heap(value)
    }
}
impl<T> Deref for Memory<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        match self {
            Self::Heap(v) => v,
            Self::Platform(v) => v.as_ref().as_ref(),
        }
    }
}
impl<T> DerefMut for Memory<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        match self {
            Self::Heap(v) => v,
            Self::Platform(v) => v.as_mut().as_mut(),
        }
    }
}

impl<T> Memory<T> {
    pub fn page_size(&self) -> usize {
        match self {
            Self::Heap(_) => 0,
            Self::Platform(v) => v.page_size(),
        }
    }
}
