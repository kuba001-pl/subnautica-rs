//! Our own reader of .NET assemblies (ECMA-335): the PE container, the
//! metadata tables and heaps, and method bodies (IL). Used to read the few
//! things Subnautica keeps only in its code (`docs/DESIGN.md` § 4.3, P1):
//! the names of the `TechType` enum, the fabricator menus built in
//! `CraftTree`, and `TechData`'s defaults. See `docs/formats/dotnet.md`.
//!
//! Pure: takes the assembly's bytes, never panics on malformed input.
//! Nothing read here is stored in the repository.

mod assembly;
#[cfg(test)]
mod encode;
mod game;
mod il;
mod reader;
mod tables;

use std::fmt;

pub use assembly::{Assembly, MemberRefInfo};
pub use game::{
    CraftNode, CraftTree, DefaultValue, const_f32, craft_trees, enum_values, field_initializer_f32,
    tech_data_defaults, tech_type_names,
};
pub use il::{Instr, Operand, decode, opcode_name};
pub use tables::{Table, TableId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// Byte offset in the assembly, or in the method body for IL errors.
    pub offset: usize,
    pub message: String,
}

impl Error {
    pub(crate) fn new(offset: usize, message: impl Into<String>) -> Error {
        Error {
            offset,
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at byte {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
