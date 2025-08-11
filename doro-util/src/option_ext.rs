pub trait OptionExt<T> {
    fn map_ext<F: FnOnce(T)>(self, f: F);
}

impl<T> OptionExt<T> for Option<T> {
    fn map_ext<F: FnOnce(T)>(self, f: F) {
        if let Some(x) = self {
            f(x)
        }
    }
}
