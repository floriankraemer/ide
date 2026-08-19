//! VT100/ANSI grid state for the embedded terminal, built on
//! `alacritty_terminal`'s `Term`/grid machinery.
//!
//! Qt-free by design (see `docs/architecture/layering.md`): this crate turns
//! a raw PTY byte stream into a renderable cell grid, cursor position, and
//! basic per-cell attributes. It does not own a PTY — `pty-core` (task F1)
//! owns the byte stream, and `ui-shell` (task F3, not yet built) wires the
//! two together, feeding bytes read from a `pty_core::PtySession` into
//! [`TerminalEmulator::feed`].

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
#[cfg(test)]
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags as CellFlags;
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Processor, Rgb};

/// Terminal size in character cells, mirroring `pty_core::PtySize` without
/// depending on `pty-core` — `terminal-core` stays a standalone emulation
/// crate (see module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSize {
    pub rows: usize,
    pub cols: usize,
}

impl GridSize {
    pub fn new(rows: usize, cols: usize) -> Self {
        Self { rows, cols }
    }
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

/// An RGB color as it should be rendered — resolved from `alacritty_terminal`'s
/// [`AnsiColor`], which can otherwise name a color indirectly (a palette
/// index or a named ANSI slot). The view (`ui-shell`, task F3) shouldn't need
/// its own copy of the default 16-color ANSI palette just to paint cells, so
/// resolution happens here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl CellColor {
    const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    fn from_ansi(color: AnsiColor, default: CellColor) -> Self {
        match color {
            AnsiColor::Spec(Rgb { r, g, b }) => CellColor::rgb(r, g, b),
            AnsiColor::Named(named) => named_color(named),
            // Indexed colors beyond the named 16 are the 256-color cube /
            // grayscale ramp; approximating those faithfully needs a full
            // palette table, which is over-engineering for a first slice.
            // Falls back to the caller's default (usually foreground/
            // background) until a real theme/palette lands.
            AnsiColor::Indexed(idx) => named_color_by_index(idx).unwrap_or(default),
        }
    }
}

fn named_color(named: NamedColor) -> CellColor {
    named_color_by_index(named as u8).unwrap_or(CellColor::rgb(229, 229, 229))
}

/// The standard 16-color ANSI palette (indices 0-15), used both for
/// [`NamedColor`] variants and low `Indexed` colors. Values match the
/// conventional xterm default palette.
fn named_color_by_index(idx: u8) -> Option<CellColor> {
    const PALETTE: [CellColor; 16] = [
        CellColor::rgb(0, 0, 0),
        CellColor::rgb(205, 0, 0),
        CellColor::rgb(0, 205, 0),
        CellColor::rgb(205, 205, 0),
        CellColor::rgb(0, 0, 238),
        CellColor::rgb(205, 0, 205),
        CellColor::rgb(0, 205, 205),
        CellColor::rgb(229, 229, 229),
        CellColor::rgb(127, 127, 127),
        CellColor::rgb(255, 0, 0),
        CellColor::rgb(0, 255, 0),
        CellColor::rgb(255, 255, 0),
        CellColor::rgb(92, 92, 255),
        CellColor::rgb(255, 0, 255),
        CellColor::rgb(0, 255, 255),
        CellColor::rgb(255, 255, 255),
    ];
    PALETTE.get(idx as usize).copied()
}

/// Basic per-cell rendering attributes exposed cheaply by `alacritty_terminal`'s
/// [`CellFlags`]. Deliberately not exhaustive (no undercurl/strikeout/dim
/// variants) — bold/italic/underline is what a first-slice grid widget needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellAttributes {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

/// One renderable cell: character plus resolved colors and attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderCell {
    pub character: char,
    pub fg: CellColor,
    pub bg: CellColor,
    pub attrs: CellAttributes,
}

/// The cursor's position in the visible grid, zero-indexed from the
/// top-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorPosition {
    pub row: usize,
    pub col: usize,
}

/// A snapshot of the terminal's visible viewport: rows of cells plus the
/// cursor position, ready to hand to a paint routine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    pub rows: Vec<Vec<RenderCell>>,
    pub cursor: CursorPosition,
}

/// No-op event sink: `Term` reports OSC/bell/clipboard-style side effects
/// through this, none of which this first slice acts on (title bar, bell,
/// clipboard integration are `ui-shell` concerns for a later task).
struct NullEventListener;

impl EventListener for NullEventListener {
    fn send_event(&self, _event: Event) {}
}

/// Owns the VT100/grid state for one terminal session. Feed it raw bytes
/// read from a `pty_core::PtySession`; read back a [`Grid`] snapshot to
/// paint.
pub struct TerminalEmulator {
    term: Term<NullEventListener>,
    parser: Processor,
}

impl TerminalEmulator {
    pub fn new(size: GridSize) -> Self {
        let term = Term::new(TermConfig::default(), &size, NullEventListener);
        Self {
            term,
            parser: Processor::new(),
        }
    }

    /// Interpret raw bytes (text and/or escape sequences) read from the PTY,
    /// updating grid/cursor state in place.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    /// Resize the grid to new dimensions, preserving content as
    /// `alacritty_terminal` reflows it (matches real terminal resize
    /// behavior: content shifts, it never panics).
    pub fn resize(&mut self, size: GridSize) {
        self.term.resize(size);
    }

    /// Snapshot the current visible viewport as renderable cells plus
    /// cursor position.
    pub fn grid(&self) -> Grid {
        let term_grid = self.term.grid();
        let cols = term_grid.columns();
        let default = CellColor::rgb(0, 0, 0);

        let mut rows: Vec<Vec<RenderCell>> = (0..term_grid.screen_lines())
            .map(|_| Vec::with_capacity(cols))
            .collect();

        for indexed in term_grid.display_iter() {
            let row_idx = (indexed.point.line.0 + term_grid.display_offset() as i32) as usize;
            let Some(row) = rows.get_mut(row_idx) else {
                continue;
            };
            let cell = indexed.cell;
            let fg = CellColor::from_ansi(cell.fg, CellColor::rgb(229, 229, 229));
            let bg = CellColor::from_ansi(cell.bg, default);
            row.push(RenderCell {
                character: cell.c,
                fg,
                bg,
                attrs: CellAttributes {
                    bold: cell.flags.intersects(CellFlags::BOLD),
                    italic: cell.flags.contains(CellFlags::ITALIC),
                    underline: cell.flags.intersects(CellFlags::ALL_UNDERLINES),
                    inverse: cell.flags.contains(CellFlags::INVERSE),
                },
            });
        }

        let cursor_point = term_grid.cursor.point;
        Grid {
            rows,
            cursor: CursorPosition {
                row: (cursor_point.line.0 + term_grid.display_offset() as i32).max(0) as usize,
                col: cursor_point.column.0,
            },
        }
    }
}

/// Move the cursor to an absolute `(line, column)` position, both
/// zero-indexed — a thin wrapper over [`Point`]/[`Line`]/[`Column`] so
/// callers/tests don't need to import `alacritty_terminal`'s index types
/// directly. Currently only used by this crate's own tests.
#[cfg(test)]
fn point(line: i32, column: usize) -> Point {
    Point::new(Line(line), Column(column))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_appears_at_expected_cell_positions() {
        let mut emulator = TerminalEmulator::new(GridSize::new(5, 20));
        emulator.feed(b"hi");

        let grid = emulator.grid();
        assert_eq!(grid.rows[0][0].character, 'h');
        assert_eq!(grid.rows[0][1].character, 'i');
        assert_eq!(grid.rows[0][2].character, ' ');
    }

    #[test]
    fn cup_escape_sequence_moves_cursor() {
        let mut emulator = TerminalEmulator::new(GridSize::new(10, 20));
        // CUP: move cursor to row 3, column 5 (1-indexed in the escape
        // sequence itself).
        emulator.feed(b"\x1b[3;5H");

        let grid = emulator.grid();
        assert_eq!(grid.cursor, CursorPosition { row: 2, col: 4 });
    }

    #[test]
    fn home_escape_sequence_moves_cursor_to_origin() {
        let mut emulator = TerminalEmulator::new(GridSize::new(10, 20));
        emulator.feed(b"hello\x1b[H");

        let grid = emulator.grid();
        assert_eq!(grid.cursor, CursorPosition { row: 0, col: 0 });
    }

    #[test]
    fn sgr_red_sets_cell_foreground() {
        let mut emulator = TerminalEmulator::new(GridSize::new(5, 20));
        emulator.feed(b"\x1b[31mR\x1b[0m");

        let grid = emulator.grid();
        let cell = grid.rows[0][0];
        assert_eq!(cell.character, 'R');
        assert_eq!(cell.fg, named_color_by_index(1).unwrap());
    }

    #[test]
    fn line_feed_advances_cursor_row() {
        let mut emulator = TerminalEmulator::new(GridSize::new(10, 20));
        emulator.feed(b"a\r\nb");

        let grid = emulator.grid();
        assert_eq!(grid.cursor.row, 1);
        assert_eq!(grid.rows[0][0].character, 'a');
        assert_eq!(grid.rows[1][0].character, 'b');
    }

    #[test]
    fn resize_does_not_panic_and_writes_still_render() {
        let mut emulator = TerminalEmulator::new(GridSize::new(10, 20));
        emulator.feed(b"before");

        emulator.resize(GridSize::new(15, 30));
        emulator.feed(b"after");

        let grid = emulator.grid();
        assert_eq!(grid.rows.len(), 15);
        assert_eq!(grid.rows[0].len(), 30);
        assert!(grid.rows.iter().flatten().any(|c| c.character == 'a'));
    }

    #[test]
    fn bold_flag_is_reflected_in_attributes() {
        let mut emulator = TerminalEmulator::new(GridSize::new(5, 20));
        emulator.feed(b"\x1b[1mB\x1b[0m");

        let grid = emulator.grid();
        assert!(grid.rows[0][0].attrs.bold);
    }

    #[test]
    fn point_helper_builds_expected_coordinates() {
        let p = point(2, 4);
        assert_eq!(p.line, Line(2));
        assert_eq!(p.column, Column(4));
    }
}
