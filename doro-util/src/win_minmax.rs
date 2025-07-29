//! win minmax 实现。其原理是，保留一个窗口时间内，1st、2nd、3rd的值，其大小依次降序。
//!
//! 详情见 [linux win_minmax](https://github.com/torvalds/linux/blob/master/lib/win_minmax.c)
use std::mem;

/// minmax 样本
#[derive(Clone, Copy, Default)]
struct MinmaxSample {
    /// 时间戳
    t: u32,

    /// 值
    v: u32,
}

#[derive(Default)]
pub struct Minmax {
    /// minmax 样本队列
    s: [MinmaxSample; 3],
}

impl Minmax {
    pub fn new() -> Self {
        Self::default()
    }

    /// 获取最大值
    pub fn minmax_get(&self) -> u32 {
        self.s[0].v
    }

    /// 更新最大值
    pub fn minmax_running_max(&mut self, win: u32, t: u32, meas: u32) -> u32 {
        let val = MinmaxSample { t, v: meas };
        if val.v >= self.s[0].v || val.t - self.s[2].t > win {
            return self.minmax_reset(t, meas);
        }

        if val.v >= self.s[1].v {
            self.s[2] = val;
            self.s[1] = val;
        } else if val.v >= self.s[2].v {
            self.s[2] = val;
        }

        self.minmax_subwin_update(win, val)
    }

    /// 更新最小值
    pub fn minmax_running_min(&mut self, win: u32, t: u32, meas: u32) -> u32 {
        let val = MinmaxSample { t, v: meas };
        if val.v <= self.s[0].v || val.t - self.s[2].t > win {
            return self.minmax_reset(t, meas);
        }

        if val.v <= self.s[1].v {
            self.s[2] = val;
            self.s[1] = val;
        } else if val.v <= self.s[2].v {
            self.s[2] = val;
        }

        self.minmax_subwin_update(win, val)
    }

    /// 更新第一、第二和第三值的 t 值（时间值）
    fn minmax_subwin_update(&mut self, win: u32, mut val: MinmaxSample) -> u32 {
        let dt = val.t - self.s[0].t;
        if dt > win {
            let mut t = val;
            mem::swap(&mut self.s[2], &mut t);
            mem::swap(&mut self.s[1], &mut t);
            mem::swap(&mut self.s[0], &mut t);
            if val.t - self.s[0].t > win {
                mem::swap(&mut self.s[1], &mut val);
                mem::swap(&mut self.s[0], &mut val);
            }
        } else if self.s[1].t == self.s[0].t && dt > win / 4 {
            self.s[2] = val;
            self.s[1] = val;
        } else if self.s[2].t == self.s[1].t && dt > win / 2 {
            self.s[2] = val;
        }
        self.s[0].v
    }

    /// 重置样本队列
    fn minmax_reset(&mut self, t: u32, meas: u32) -> u32 {
        let m = MinmaxSample { t, v: meas };
        self.s[0] = m;
        self.s[1] = m;
        self.s[2] = m;
        meas
    }
}
