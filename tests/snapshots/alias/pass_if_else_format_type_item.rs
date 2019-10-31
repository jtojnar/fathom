// This file is automatically @generated by ddl 0.1.0
// It is not intended for manual editing.

pub const FOO: bool = true;

#[derive(Copy, Clone)]
pub struct Test {
    inner: ddl_rt::Either<f64, f32>,
}

impl Test {
    pub fn inner(&self) -> ddl_rt::Either<f64, f32> {
        self.inner
    }
}

impl ddl_rt::Format for Test {
    type Host = Test;
}

impl<'data> ddl_rt::ReadFormat<'data> for Test {
    fn read(reader: &mut ddl_rt::FormatReader<'data>) -> Result<Test, ddl_rt::ReadError> {
        let inner = if FOO { ddl_rt::Either::Left(reader.read::<ddl_rt::F64Be>()?) } else { ddl_rt::Either::Right(reader.read::<ddl_rt::F32Be>()?) };

        Ok(Test {
            inner,
        })
    }
}
