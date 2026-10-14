//! The human-output boundary. Escaping is stateless across formatting fragments
//! and model chunks: no fragment can emit an ESC or another terminal action.

use std::fmt::{self, Write as _};

pub(crate) struct Safe<T>(pub T);

impl<T: fmt::Display> fmt::Display for Safe<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct Escaped<'a, 'b>(&'a mut fmt::Formatter<'b>);
        impl fmt::Write for Escaped<'_, '_> {
            fn write_str(&mut self, text: &str) -> fmt::Result {
                for ch in text.chars() {
                    if ch.is_control() && ch != '\n' && ch != '\t' {
                        std::write!(self.0, "\\u{:04x}", u32::from(ch))?;
                    } else {
                        self.0.write_char(ch)?;
                    }
                }
                Ok(())
            }
        }
        std::write!(Escaped(f), "{}", self.0)
    }
}

// Local macros apply the same boundary to entire formatted human messages, not
// just selected model fields. JSON-specific output should use its own serializer.
macro_rules! safe_println {
    ($($arg:tt)*) => { std::println!("{}", crate::safe::Safe(format_args!($($arg)*))) };
}
macro_rules! safe_write {
    ($dst:expr, $($arg:tt)*) => {
        std::write!($dst, "{}", crate::safe::Safe(format_args!($($arg)*)))
    };
}
macro_rules! safe_writeln {
    ($dst:expr) => { std::writeln!($dst) };
    ($dst:expr, $($arg:tt)*) => {
        std::writeln!($dst, "{}", crate::safe::Safe(format_args!($($arg)*)))
    };
}

pub(crate) use {safe_println, safe_write, safe_writeln};

#[cfg(test)]
mod tests;
