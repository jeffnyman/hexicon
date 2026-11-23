//! Banner windows: the `banner` functions of the `tads-io` function set.
//!
//! Each banner has a formatter of its own, as each has a console of its
//! own in the reference VM (`CVmConsoleBanner`), and the host is handed
//! the formatted text. Where the host has no banners, as a plain console
//! has none, creating one fails, and programs carry on without.

use hexicon_host::banner::{BannerSpec, Placement, kind, style, units};

use crate::console::{Formatter, Newline};
use crate::error::{VmError, VmResult, code};
use crate::value::Value;
use crate::vm::Vm;

// Where a banner goes in its parent's list, as programs give it.
const BANNER_FIRST: i32 = 1;
const BANNER_BEFORE: i32 = 3;
const BANNER_AFTER: i32 = 4;

/// A banner the program has created.
#[derive(Debug)]
pub(crate) struct Banner {
    /// The host's handle for it.
    host: u32,
    kind: i32,
    formatter: Formatter,
}

fn bad_value() -> VmError {
    VmError::new(code::BAD_VAL_BIF)
}

impl Vm {
    /// Run a banner function, by its index in the `tads-io` set.
    pub(crate) fn bif_banner(&mut self, index: u16, argc: u32) -> VmResult<()> {
        match index {
            // bannerCreate(parent?, where, other, windowType, align, size,
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
                    BANNER_BEFORE | BANNER_AFTER => Some(self.int_arg(arg(self, 1))?),
                    _ => None,
                };
                let window_type = self.int_arg(arg(self, 2))?;
                let align = self.int_arg(arg(self, 3))?;
                let (size, size_units) = match arg(self, 4) {
                    Value::Nil => (0, units::CELLS),
                    value => (self.int_arg(value)?, self.int_arg(arg(self, 5))?),
                };
                let flags = self.int_arg(arg(self, 6))? as u32;

                // A handle that is not a banner's means the main window,
                // or, for the banner to go next to, none.
                let parent = parent.and_then(|handle| self.banner_host(handle));
                let other = other.and_then(|handle| self.banner_host(handle));
                let placement = match (place, other) {
                    (BANNER_FIRST, _) => Placement::First,
                    (BANNER_BEFORE, Some(other)) => Placement::Before(other),
                    (BANNER_AFTER, Some(other)) => Placement::After(other),
                    (BANNER_BEFORE | BANNER_AFTER, None) => Placement::Before(0),
                    // The end of the list (2), and anything else.
                    _ => Placement::Last,
                };
                let spec = BannerSpec {
                    kind: window_type,
                    align,
                    size,
                    units: size_units,
                    style: flags,
                };
                self.r0 = match self.host.banner_create(parent, placement, spec) {
                    Some(host) => {
                        let formatter = if window_type == kind::GRID {
                            Formatter::grid()
                        } else {
                            Formatter::new(self.banner_width(host))
                        };
                        let banner = Banner {
                            host,
                            kind: window_type,
                            formatter,
                        };
                        // Handles are given out from one, lowest free first.
                        let slot = match self.io.banners.iter().position(Option::is_none) {
                            Some(slot) => slot,
                            None => {
                                self.io.banners.push(None);
                                self.io.banners.len() - 1
                            }
                        };
                        self.io.banners[slot] = Some(banner);
                        Value::Int(slot as i32 + 1)
                    }
                    None => Value::Nil,
                };
            }
            // bannerDelete(handle): a handle that is not a banner's is
            // ignored.
            20 => {
                Self::check_argc(argc, 1)?;
                let handle = self.int_arg(self.arg(0))?;
                if let Some(slot) = self.banner_slot(handle) {
                    if let Some(banner) = self.io.banners[slot].take() {
                        self.host.banner_delete(banner.host);
                    }
                }
            }
            // bannerClear(handle)
            21 => {
                Self::check_argc(argc, 1)?;
                let slot = self.banner_arg(self.arg(0))?;
                self.flush_banner(slot, Newline::None);
                let banner = self.io.banners[slot].as_mut().expect("banner");
                banner.formatter.empty_buffers();
                let host = banner.host;
                self.host.banner_clear(host);
            }
            // bannerSay(handle, ...)
            22 => {
                Self::check_argc_range(argc, 1, 32767)?;
                let slot = self.banner_arg(self.arg(0))?;
                for index in 1..argc as usize {
                    let Some(text) = self.say_text(self.arg(index))? else {
                        continue;
                    };
                    self.sync_banner_width(slot);
                    let banner = self.io.banners[slot].as_mut().expect("banner");
                    banner.formatter.format_text(&text);
                    self.send_banner(slot);
                }
            }
            // bannerFlush(handle)
            23 => {
                Self::check_argc(argc, 1)?;
                let slot = self.banner_arg(self.arg(0))?;
                self.flush_banner(slot, Newline::None);
                let host = self.io.banners[slot].as_ref().expect("banner").host;
                self.host.banner_flush(host);
            }
            // bannerSizeToContents(handle)
            24 => {
                Self::check_argc(argc, 1)?;
                let slot = self.banner_arg(self.arg(0))?;
                self.flush_banner(slot, Newline::None);
                let host = self.io.banners[slot].as_ref().expect("banner").host;
                self.host.banner_size_to_contents(host);
            }
            // bannerGoTo(handle, row, col)
            25 => {
                Self::check_argc(argc, 3)?;
                let handle = self.arg(0);
                let row = self.int_arg(self.arg(1))?;
                let column = self.int_arg(self.arg(2))?;
                if row < 1 || column < 1 {
                    return Err(bad_value());
                }
                let slot = self.banner_arg(handle)?;
                // What is waiting to be written is not to appear at the new
                // position.
                self.flush_banner(slot, Newline::None);
                let banner = self.io.banners[slot].as_mut().expect("banner");
                banner.formatter.empty_buffers();
                let host = banner.host;
                self.host
                    .banner_go_to(host, row as usize - 1, column as usize - 1);
            }
            // bannerSetTextColor(handle, fg, bg): the text is plain.
            26 => {
                Self::check_argc(argc, 3)?;
                let handle = self.arg(0);
                self.int_arg(self.arg(1))?;
                self.int_arg(self.arg(2))?;
                self.banner_arg(handle)?;
            }
            // bannerSetScreenColor(handle, color)
            27 => {
                Self::check_argc(argc, 2)?;
                let handle = self.arg(0);
                let color = self.int_arg(self.arg(1))? as u32;
                let slot = self.banner_arg(handle)?;
                let host = self.io.banners[slot].as_ref().expect("banner").host;
                self.host.banner_set_screen_color(host, color);
            }
            // bannerGetInfo(handle)
            28 => {
                Self::check_argc(argc, 1)?;
                let slot = self.banner_arg(self.arg(0))?;
                let banner = self.io.banners[slot].as_ref().expect("banner");
                let (host, banner_kind) = (banner.host, banner.kind);
                self.r0 = match self.host.banner_info(host) {
                    Some(info) => {
                        let mut flags = info.style;
                        // A grid has no markup, so no <tab>.
                        if banner_kind == kind::GRID {
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
            // bannerSetSize(handle, size, sizeUnits, isAdvisory)
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
                let slot = self.banner_arg(handle)?;
                let host = self.io.banners[slot].as_ref().expect("banner").host;
                self.host.banner_set_size(host, size, size_units, advisory);
            }
            _ => unreachable!("not a banner function"),
        }
        Ok(())
    }

    /// The slot of the banner a handle names, if it names one.
    fn banner_slot(&self, handle: i32) -> Option<usize> {
        usize::try_from(handle)
            .ok()
            .and_then(|handle| handle.checked_sub(1))
            .filter(|&slot| matches!(self.io.banners.get(slot), Some(Some(_))))
    }

    /// The host's handle for the banner a program's handle names.
    fn banner_host(&self, handle: i32) -> Option<u32> {
        self.banner_slot(handle)
            .and_then(|slot| self.io.banners[slot].as_ref())
            .map(|banner| banner.host)
    }

    /// The slot of the banner an argument names, or an error.
    fn banner_arg(&self, value: Value) -> VmResult<usize> {
        let handle = self.int_arg(value)?;
        self.banner_slot(handle).ok_or_else(bad_value)
    }

    /// The width a text banner's lines are broken to.
    fn banner_width(&mut self, host: u32) -> usize {
        self.host
            .banner_info(host)
            .map_or(crate::load::CONSOLE_WIDTH, |info| info.columns)
    }

    /// Take a text banner's width from the host, which changes it as the
    /// banner and the screen change size.
    fn sync_banner_width(&mut self, slot: usize) {
        let banner = self.io.banners[slot].as_ref().expect("banner");
        if banner.kind == kind::GRID {
            return;
        }
        let width = self.banner_width(banner.host);
        if let Some(banner) = self.io.banners[slot].as_mut() {
            banner.formatter.set_width(width);
        }
    }

    /// Hand a banner's finished text to the host, and any background
    /// color asked for with `<body>`.
    fn send_banner(&mut self, slot: usize) {
        let banner = self.io.banners[slot].as_mut().expect("banner");
        let output = banner.formatter.take_output();
        let color = banner.formatter.take_body_color();
        let host = banner.host;
        if let Some(color) = color {
            self.host.banner_set_screen_color(host, color);
        }
        if !output.is_empty() {
            self.host.banner_write(host, &output);
        }
    }

    /// Write out a banner's unfinished line.
    fn flush_banner(&mut self, slot: usize, newline: Newline) {
        if let Some(banner) = self.io.banners[slot].as_mut() {
            banner.formatter.flush(newline);
            self.send_banner(slot);
        }
    }

    /// Write out every banner's unfinished line, as the reference does
    /// whenever it flushes everything, as before reading input.
    pub(crate) fn flush_banners(&mut self) {
        for slot in 0..self.io.banners.len() {
            if self.io.banners[slot].is_some() {
                self.flush_banner(slot, Newline::None);
            }
        }
    }
}
