use super::Table;

impl Table {
    /// Return the bytes of the two vectors of the key index for the measurement of #539.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Table::new(ProcessId(0), Access::Public, 1, TableType::Set).index_bytes(), 0);
    /// ```
    pub(in crate::runtime) fn index_bytes(&self) -> usize {
        self.index.hashes.capacity() * std::mem::size_of::<u64>()
            + self.index.slots.capacity() * std::mem::size_of::<usize>()
    }
}
