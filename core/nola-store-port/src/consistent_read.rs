// SPDX-License-Identifier: Apache-2.0
//! Scope one aggregate read without exposing SQL or a concrete database.
/// Every read through `reader` observes one committed snapshot. Existing outer
/// receipt transactions are reused. Success or failure releases only this read
/// scope; it must never commit or roll back the caller's outer transaction.
pub trait ConsistentReadRepository {
    fn with_consistent_read<T>(
        &mut self,
        reader: impl FnOnce(&Self) -> Result<T, String>,
    ) -> Result<T, String>;

    fn with_consistent_read_mut<T>(
        &mut self,
        reader: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String>
    where
        Self: Sized,
    {
        reader(self)
    }
}
