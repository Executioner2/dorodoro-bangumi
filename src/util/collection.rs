use std::collections::VecDeque;

pub struct FixedQueue<T> {
    queue: VecDeque<T>,
    limit: usize,
}

impl<T> FixedQueue<T> {
    pub fn new(limit: usize) -> Self {
        Self {
            queue: VecDeque::new(),
            limit,
        }
    }

    pub fn push(&mut self, item: T) {
        if self.queue.len() == self.limit {
            self.queue.pop_front();
        }
        self.queue.push_back(item);
    }

    pub fn iter(&self) -> std::collections::vec_deque::Iter<T> {
        self.queue.iter()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }
}
