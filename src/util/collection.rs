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

    pub fn push(&mut self, item: T) -> Option<T> {
        let mut res = None;
        if self.queue.len() == self.limit {
            res = self.queue.pop_front();
        }
        self.queue.push_back(item);
        res
    }

    pub fn iter(&self) -> std::collections::vec_deque::Iter<T> {
        self.queue.iter()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }
}
