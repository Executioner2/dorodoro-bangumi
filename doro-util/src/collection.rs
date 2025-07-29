use std::collections::VecDeque;

#[derive(Default, Debug)]
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

    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn iter(&self) -> std::collections::vec_deque::Iter<T> {
        self.queue.iter()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn clear(&mut self) {
        self.queue.clear();
    }

    pub fn back(&self) -> Option<&T> {
        self.queue.back()
    }

    pub fn pop_front(&mut self) -> Option<T> {
        self.queue.pop_front()
    }

    pub fn peek_front(&self) -> Option<&T> {
        self.queue.front()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}
