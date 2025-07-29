use crate::torrent::{File, Info};

#[test]
fn test_calculate_file_piece() {
    let mut files = vec![
        File::new(10, vec![], None),
        File::new(10, vec![], None),
        File::new(11, vec![], None),
        File::new(10, vec![], None),
    ];
    let res = Info::calculate_file_piece(&mut files, 10);
    assert_eq!(res, vec![(0, 0), (1, 1), (2, 3), (3, 4)]);
}

#[test]
fn test_find_intervals() {
    fn find_intervals(ars: &[(i32, i32)], x: i32) -> &[(i32, i32)] {
        if ars.is_empty() {
            return &[];
        }

        // 找到第一个右端点 >= x 的区间索引
        let m_low = ars.partition_point(|&(_, r)| r < x);

        // 检查是否越界或左端点超过 x
        if m_low >= ars.len() || ars[m_low].0 > x {
            return &[];
        }

        // 找到第一个左端点 > x 的区间索引
        let n_high = ars.partition_point(|&(l, _)| l <= x);

        // 确定有效范围并返回切片
        &ars[m_low..n_high]
    }

    let ars = [(1, 3), (3, 5), (6, 8), (8, 10), (11, 21)];

    assert_eq!(&[(1, 3), (3, 5)], find_intervals(&ars, 3));
    assert_eq!(&[(3, 5)], find_intervals(&ars, 5));
    assert_eq!(&[(6, 8), (8, 10)], find_intervals(&ars, 8));
    assert_eq!(&[(11, 21)], find_intervals(&ars, 21));
    assert_eq!(&[] as &[(i32, i32)], find_intervals(&ars, 0));
    assert_eq!(&[] as &[(i32, i32)], find_intervals(&ars, 22));
}
