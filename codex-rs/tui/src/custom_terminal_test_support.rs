//! Inspect the completed frame without adding test helpers to the terminal implementation.

use std::io;
use std::io::Write;

use ratatui::backend::Backend;
use ratatui::buffer::Buffer;

use super::Terminal;

pub(crate) fn last_rendered_buffer<B>(terminal: &Terminal<B>) -> &Buffer
where
    B: Backend<Error = io::Error> + Write,
{
    terminal.previous_buffer()
}
