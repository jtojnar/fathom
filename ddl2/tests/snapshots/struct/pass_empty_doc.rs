// This file is automatically @generated by ddl 0.1.0
// It is not intended for manual editing.

/// This is an empty struct.
///
/// It will not consume any input.
#[derive(Copy, Clone)]
pub struct Empty {}

impl ddl_rt::Binary for Empty {
    type Host = Empty;
}

impl<'data> ddl_rt::ReadBinary<'data> for Empty {
    fn read(_: &mut ddl_rt::ReadCtxt<'data>) -> Result<Empty, ddl_rt::ReadError> {
        Ok(Empty {})
    }
}
