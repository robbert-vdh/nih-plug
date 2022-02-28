use std::ops::Deref;

/// Send+Sync wrapper around CLAP host extension pointers.
pub struct ClapPtr<T> {
    inner: *const T,
}

impl<T> Deref for ClapPtr<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.inner }
    }
}

unsafe impl<T> Send for ClapPtr<T> {}
unsafe impl<T> Sync for ClapPtr<T> {}

impl<T> ClapPtr<T> {
    /// Create a wrapper around a CLAP object pointer.
    ///
    /// # Safety
    ///
    /// The pointer must point to a valid object with a lifetime that exceeds this object.
    pub unsafe fn new(ptr: *const T) -> Self {
        Self { inner: ptr }
    }
}
