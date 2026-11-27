//! Panes: the `pane` functions of the `hexicon-io` function set.
//!
//! Each pane has a formatter of its own, as each has a console of its
//! own in the reference VM (`CVmConsolePane`), and the host is handed
//! the formatted text. Where the host has no panes, as a plain console
//! has none, creating one fails, and programs carry on without.

use hexicon_host::pane::{PaneSpec, Placement, kind, style, units};

use crate::console::{Formatter, Newline};
use crate::error::{VmError, VmResult, code};
use crate::value::Value;
use crate::vm::Vm;

// Where a pane goes in its parent's list, as programs give it.
const PANE_FIRST: i32 = 1;
const PANE_BEFORE: i32 = 3;
const PANE_AFTER: i32 = 4;

/// A pane the program has created.
#[derive(Debug)]
pub(crate) struct Pane {
    /// The host's handle for it.
    host: u32,
    kind: i32,
    formatter: Formatter,
}

fn bad_value() -> VmError {
    VmError::new(code::BAD_VAL_BIF)
}

impl Vm {
    /// Run a pane function, by its index in the `hexicon-io` set.
    pub(crate) fn bif_pane(&mut self, index: u16, argc: u32) -> VmResult<()> {
        match index {
            // paneCreate(parent?, where, other, windowType, align, size,
            // sizeUnits, styleFlags)
            19 => {
                Self::check_argc_range(argc, 7, 8)?;
                // Without a parent, the arguments are the older form.
                let (parent, first) = if argc == 7 {
                    (None, 0)
                } else {
                    match self.arg(0) {
                        Value::Nil => (None, 1),
                        value => (Some(self.int_arg(value)?), 1),
                    }
                };
                let arg = |vm: &Self, n: usize| vm.arg(first + n);
                let place = self.int_arg(arg(self, 0))?;
                let other = match place {
                    PANE_BEFORE | PANE_AFTER => Some(self.int_arg(arg(self, 1))?),
                    _ => None,
                };
                let window_type = self.int_arg(arg(self, 2))?;
                let align = self.int_arg(arg(self, 3))?;
                let (size, size_units) = match arg(self, 4) {
                    Value::Nil => (0, units::CELLS),
                    value => (self.int_arg(value)?, self.int_arg(arg(self, 5))?),
                };
                let flags = self.int_arg(arg(self, 6))? as u32;

                // A handle that is not a pane's means the main window,
                // or, for the pane to go next to, none.
                let parent = parent.and_then(|handle| self.pane_host(handle));
                let other = other.and_then(|handle| self.pane_host(handle));
                let placement = match (place, other) {
                    (PANE_FIRST, _) => Placement::First,
                    (PANE_BEFORE, Some(other)) => Placement::Before(other),
                    (PANE_AFTER, Some(other)) => Placement::After(other),
                    (PANE_BEFORE | PANE_AFTER, None) => Placement::Before(0),
                    // The end of the list (2), and anything else.
                    _ => Placement::Last,
                };
                let spec = PaneSpec {
                    kind: window_type,
                    align,
                    size,
                    units: size_units,
                    style: flags,
                };
                self.r0 = match self.host.pane_create(parent, placement, spec) {
                    Some(host) => {
                        let formatter = if window_type == kind::GRID {
                            Formatter::grid()
                        } else {
                            Formatter::new(self.pane_width(host))
                        };
                        let pane = Pane {
                            host,
                            kind: window_type,
                            formatter,
                        };
                        // Handles are given out from one, lowest free first.
                        let slot = match self.io.panes.iter().position(Option::is_none) {
                            Some(slot) => slot,
                            None => {
                                self.io.panes.push(None);
                                self.io.panes.len() - 1
                            }
                        };
                        self.io.panes[slot] = Some(pane);
                        Value::Int(slot as i32 + 1)
                    }
                    None => Value::Nil,
                };
            }
            // paneDelete(handle): a handle that is not a pane's is
            // ignored.
            20 => {
                Self::check_argc(argc, 1)?;
                let handle = self.int_arg(self.arg(0))?;
                if let Some(slot) = self.pane_slot(handle) {
                    if let Some(pane) = self.io.panes[slot].take() {
                        self.host.pane_delete(pane.host);
                    }
                }
            }
            // paneClear(handle)
            21 => {
                Self::check_argc(argc, 1)?;
                let slot = self.pane_arg(self.arg(0))?;
                self.flush_pane(slot, Newline::None);
                let pane = self.io.panes[slot].as_mut().expect("pane");
                pane.formatter.empty_buffers();
                let host = pane.host;
                self.host.pane_clear(host);
            }
            // paneSay(handle, ...)
            22 => {
                Self::check_argc_range(argc, 1, 32767)?;
                let slot = self.pane_arg(self.arg(0))?;
                let rich = self.host.rich_display().is_some()
                    && self.io.panes[slot].as_ref().expect("pane").kind != kind::GRID;
                for index in 1..argc as usize {
                    let Some(text) = self.say_text(self.arg(index))? else {
                        continue;
                    };
                    if rich {
                        // A rich display takes the pane's markup as it is.
                        let host = self.io.panes[slot].as_ref().expect("pane").host;
                        self.host
                            .pane_write_markup(host, &String::from_utf8_lossy(&text));
                        continue;
                    }
                    self.sync_pane_width(slot);
                    let pane = self.io.panes[slot].as_mut().expect("pane");
                    pane.formatter.format_text(&text);
                    self.send_pane(slot);
                }
            }
            // paneFlush(handle)
            23 => {
                Self::check_argc(argc, 1)?;
                let slot = self.pane_arg(self.arg(0))?;
                self.flush_pane(slot, Newline::None);
                let host = self.io.panes[slot].as_ref().expect("pane").host;
                self.host.pane_flush(host);
            }
            // paneSizeToContents(handle)
            24 => {
                Self::check_argc(argc, 1)?;
                let slot = self.pane_arg(self.arg(0))?;
                self.flush_pane(slot, Newline::None);
                let host = self.io.panes[slot].as_ref().expect("pane").host;
                self.host.pane_size_to_contents(host);
            }
            // paneGoTo(handle, row, col)
            25 => {
                Self::check_argc(argc, 3)?;
                let handle = self.arg(0);
                let row = self.int_arg(self.arg(1))?;
                let column = self.int_arg(self.arg(2))?;
                if row < 1 || column < 1 {
                    return Err(bad_value());
                }
                let slot = self.pane_arg(handle)?;
                // What is waiting to be written is not to appear at the new
                // position.
                self.flush_pane(slot, Newline::None);
                let pane = self.io.panes[slot].as_mut().expect("pane");
                pane.formatter.empty_buffers();
                let host = pane.host;
                self.host
                    .pane_go_to(host, row as usize - 1, column as usize - 1);
            }
            // paneSetTextColor(handle, fg, bg): the text is plain.
            26 => {
                Self::check_argc(argc, 3)?;
                let handle = self.arg(0);
                self.int_arg(self.arg(1))?;
                self.int_arg(self.arg(2))?;
                self.pane_arg(handle)?;
            }
            // paneSetScreenColor(handle, color)
            27 => {
                Self::check_argc(argc, 2)?;
                let handle = self.arg(0);
                let color = self.int_arg(self.arg(1))? as u32;
                let slot = self.pane_arg(handle)?;
                let host = self.io.panes[slot].as_ref().expect("pane").host;
                self.host.pane_set_screen_color(host, color);
            }
            // paneGetInfo(handle)
            28 => {
                Self::check_argc(argc, 1)?;
                let slot = self.pane_arg(self.arg(0))?;
                let pane = self.io.panes[slot].as_ref().expect("pane");
                let (host, pane_kind) = (pane.host, pane.kind);
                self.r0 = match self.host.pane_info(host) {
                    Some(info) => {
                        let mut flags = info.style;
                        // A grid has no markup, so no <tab>.
                        if pane_kind == kind::GRID {
                            flags &= !style::TAB_ALIGN;
                        }
                        self.new_list(vec![
                            Value::Int(info.align),
                            Value::Int(flags as i32),
                            Value::Int(info.rows as i32),
                            Value::Int(info.columns as i32),
                            Value::Int(0),
                            Value::Int(0),
                        ])
                    }
                    None => Value::Nil,
                };
            }
            // paneSetSize(handle, size, sizeUnits, isAdvisory)
            29 => {
                Self::check_argc(argc, 4)?;
                let handle = self.arg(0);
                let size = self.int_arg(self.arg(1))?;
                let size_units = self.int_arg(self.arg(2))?;
                let advisory = match self.arg(3) {
                    Value::Nil => false,
                    Value::True => true,
                    Value::Int(n) => n != 0,
                    _ => return Err(VmError::new(code::BAD_TYPE_BIF)),
                };
                let slot = self.pane_arg(handle)?;
                let host = self.io.panes[slot].as_ref().expect("pane").host;
                self.host.pane_set_size(host, size, size_units, advisory);
            }
            _ => unreachable!("not a pane function"),
        }
        Ok(())
    }

    /// The slot of the pane a handle names, if it names one.
    fn pane_slot(&self, handle: i32) -> Option<usize> {
        usize::try_from(handle)
            .ok()
            .and_then(|handle| handle.checked_sub(1))
            .filter(|&slot| matches!(self.io.panes.get(slot), Some(Some(_))))
    }

    /// The host's handle for the pane a program's handle names.
    fn pane_host(&self, handle: i32) -> Option<u32> {
        self.pane_slot(handle)
            .and_then(|slot| self.io.panes[slot].as_ref())
            .map(|pane| pane.host)
    }

    /// The slot of the pane an argument names, or an error.
    fn pane_arg(&self, value: Value) -> VmResult<usize> {
        let handle = self.int_arg(value)?;
        self.pane_slot(handle).ok_or_else(bad_value)
    }

    /// The width a text pane's lines are broken to.
    fn pane_width(&mut self, host: u32) -> usize {
        self.host
            .pane_info(host)
            .map_or(crate::load::CONSOLE_WIDTH, |info| info.columns)
    }

    /// Take a text pane's width from the host, which changes it as the
    /// pane and the screen change size.
    fn sync_pane_width(&mut self, slot: usize) {
        let pane = self.io.panes[slot].as_ref().expect("pane");
        if pane.kind == kind::GRID {
            return;
        }
        let width = self.pane_width(pane.host);
        if let Some(pane) = self.io.panes[slot].as_mut() {
            pane.formatter.set_width(width);
        }
    }

    /// Hand a pane's finished text to the host, and any background
    /// color asked for with `<body>`.
    fn send_pane(&mut self, slot: usize) {
        let pane = self.io.panes[slot].as_mut().expect("pane");
        let output = pane.formatter.take_output();
        let color = pane.formatter.take_body_color();
        let host = pane.host;
        if let Some(color) = color {
            self.host.pane_set_screen_color(host, color);
        }
        if !output.is_empty() {
            self.host.pane_write(host, &output);
        }
    }

    /// Write out a pane's unfinished line.
    fn flush_pane(&mut self, slot: usize, newline: Newline) {
        if let Some(pane) = self.io.panes[slot].as_mut() {
            pane.formatter.flush(newline);
            self.send_pane(slot);
        }
    }

    /// Write out every pane's unfinished line, as the reference does
    /// whenever it flushes everything, as before reading input.
    pub(crate) fn flush_panes(&mut self) {
        for slot in 0..self.io.panes.len() {
            if self.io.panes[slot].is_some() {
                self.flush_pane(slot, Newline::None);
            }
        }
    }
}
