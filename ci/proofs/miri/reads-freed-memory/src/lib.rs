//! A use after free, which Miri refuses.

#[test]
fn reads_a_byte_it_freed() {
    let byte = Box::into_raw(Box::new(7_u8));
    // SAFETY: none: the fixture reads the byte after freeing it, on purpose.
    let read = unsafe {
        drop(Box::from_raw(byte));
        byte.read()
    };
    assert!(read < u8::MAX);
}
