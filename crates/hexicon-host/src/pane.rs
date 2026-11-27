//! Panes: the panes a program can divide the screen into, such as
//! a status line across the top.
//!
//! A host that displays panes keeps a [`Screen`], which does what the
//! reference interpreter's character-mode display layer does (`osgen3.c`):
//! it keeps the tree of panes, lays them out in character cells, holds
//! the text each one shows, and works out how big a pane's contents
//! are. The host only draws what the screen says is where.
//!
//! The VM formats the text for a pane before handing it over, broken
//! into lines that fit the pane's width, so a pane receives plain
//! text with newlines.

use std::collections::{BTreeMap, BTreeSet};

/// Where a pane goes in its parent's list of panes. Panes earlier in
/// the list take their space first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// At the start of the list.
    First,
    /// At the end of the list.
    Last,
    /// Just before another pane, or at the end if it is not there.
    Before(u32),
    /// Just after another pane, or at the end if it is not there.
    After(u32),
}

/// The kinds of pane.
pub mod kind {
    /// Text that flows from line to line, as in the main window.
    pub const TEXT: i32 = 1;
    /// A grid of character cells, written at a position.
    pub const GRID: i32 = 2;
}

/// Which side of its parent a pane is attached to.
pub mod align {
    /// The top, across the full width.
    pub const TOP: i32 = 0;
    /// The bottom, across the full width.
    pub const BOTTOM: i32 = 1;
    /// The left, the full height.
    pub const LEFT: i32 = 2;
    /// The right, the full height.
    pub const RIGHT: i32 = 3;
}

/// How a pane's size is given.
pub mod units {
    /// A percentage of the parent's size.
    pub const PERCENT: i32 = 1;
    /// Character cells: lines for a top or bottom pane, columns for a
    /// left or right one.
    pub const CELLS: i32 = 2;
}

/// Pane style flags. The values are the ones programs see.
pub mod style {
    /// A border between the pane and the rest of the screen.
    pub const BORDER: u32 = 0x01;
    /// A vertical scroll bar.
    pub const VSCROLL: u32 = 0x02;
    /// A horizontal scroll bar.
    pub const HSCROLL: u32 = 0x04;
    /// Keep the last line in view as text is added.
    pub const AUTO_VSCROLL: u32 = 0x08;
    /// Keep the end of the line in view as text is added.
    pub const AUTO_HSCROLL: u32 = 0x10;
    /// `<tab>` alignment works, as it does on a fixed-pitch display.
    pub const TAB_ALIGN: u32 = 0x20;
    /// Pause when a screenful has gone by.
    pub const MORE_MODE: u32 = 0x40;
    /// Sizing the parent to its contents makes room for this pane's
    /// width.
    pub const HSTRUT: u32 = 0x80;
    /// Sizing the parent to its contents makes room for this pane's
    /// height.
    pub const VSTRUT: u32 = 0x100;
}

/// Colors the host chooses, as programs name them. Any other color is an
/// RGB value, `0xrrggbb`.
pub mod color {
    /// The color of ordinary text.
    pub const TEXT: u32 = 0x0200_0000;
    /// The background of ordinary text.
    pub const TEXT_BG: u32 = 0x0300_0000;
    /// The color of the status line's text.
    pub const STATUS_TEXT: u32 = 0x0400_0000;
    /// The status line's background.
    pub const STATUS_BG: u32 = 0x0500_0000;
}

/// What a program asks for when it creates a pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneSpec {
    /// A [`kind`] value.
    pub kind: i32,
    /// An [`align`] value.
    pub align: i32,
    /// The size, in `units`.
    pub size: i32,
    /// A [`units`] value.
    pub units: i32,
    /// A combination of [`style`] flags.
    pub style: u32,
}

/// What a program can learn about a pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneInfo {
    /// An [`align`] value.
    pub align: i32,
    /// The [`style`] flags in effect.
    pub style: u32,
    /// The height in lines.
    pub rows: usize,
    /// The width in columns.
    pub columns: usize,
}

/// An area of the screen, in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    /// The leftmost column.
    pub x: usize,
    /// The top line.
    pub y: usize,
    /// The width in columns.
    pub width: usize,
    /// The height in lines.
    pub height: usize,
}

/// How a pane is placed, as the program asked: for a display that lays
/// the panes out in its own measures, as a rich one does, and not in
/// character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneLayout {
    /// The pane it is part of, or `None` for the main window.
    pub parent: Option<u32>,
    /// A [`kind`] value.
    pub kind: i32,
    /// An [`align`] value.
    pub align: i32,
    /// The size, in `units`.
    pub size: i32,
    /// A [`units`] value.
    pub units: i32,
    /// Whether the program last asked for it to be sized to what it
    /// holds, rather than given a size.
    pub fitted: bool,
}

/// The most lines of text a pane keeps. A pane shows no more than a
/// screenful, so more would only be kept to be thrown away.
const MAX_LINES: usize = 500;

#[derive(Debug)]
struct Window {
    /// The pane it is part of, or `None` for the main window.
    parent: Option<u32>,
    /// Whether it is in its parent's list, and so on the screen if its
    /// parent is. A pane whose parent is deleted stays, but is shown
    /// nowhere.
    attached: bool,
    children: Vec<u32>,
    kind: i32,
    align: i32,
    size: i32,
    units: i32,
    flags: u32,
    /// The text, a line at a time.
    lines: Vec<Vec<char>>,
    /// Where the next character goes.
    x: usize,
    y: usize,
    /// The furthest column and line written to, for sizing to contents.
    xmax: usize,
    ymax: usize,
    /// Where the pane is on the screen.
    rect: Rect,
    /// The background color asked for.
    screen_color: Option<u32>,
    /// Whether it was last sized to its contents.
    fitted: bool,
}

impl Window {
    fn clear(&mut self) {
        self.lines.clear();
        self.x = 0;
        self.y = 0;
        self.xmax = 0;
        self.ymax = 0;
    }

    fn horizontal(&self) -> bool {
        self.align == align::TOP || self.align == align::BOTTOM
    }

    fn put(&mut self, c: char) {
        while self.lines.len() <= self.y {
            self.lines.push(Vec::new());
        }
        let line = &mut self.lines[self.y];
        while line.len() < self.x {
            line.push(' ');
        }
        if self.x < line.len() {
            line[self.x] = c;
        } else {
            line.push(c);
        }
        self.xmax = self.xmax.max(self.x);
        self.x += 1;
    }

    fn newline(&mut self) {
        self.ymax = self.ymax.max(self.y);
        self.y += 1;
        self.x = 0;
        if self.kind == kind::TEXT && self.y >= MAX_LINES {
            // Forget the oldest line. What is in view, and the size of
            // the contents, are measured from the end.
            self.lines.remove(0);
            self.y -= 1;
            self.ymax = self.ymax.saturating_sub(1);
        }
    }
}

/// The panes on a screen, and where everything goes.
#[derive(Debug)]
pub struct Screen {
    rows: usize,
    columns: usize,
    /// Whether a pane can be at the left or right of the main window.
    /// A terminal cannot keep the main window's text to part of its width,
    /// so it says no.
    side_panes: bool,
    windows: BTreeMap<u32, Window>,
    /// The panes in the main window, in order.
    main_children: Vec<u32>,
    next_id: u32,
    main: Rect,
    /// The old-style status line, kept as a pane of its own.
    status: Option<u32>,
    layout_changed: bool,
    changed: BTreeSet<u32>,
}

impl Screen {
    /// A screen of the given size, with no panes.
    pub fn new(rows: usize, columns: usize, side_panes: bool) -> Self {
        Self {
            rows,
            columns,
            side_panes,
            windows: BTreeMap::new(),
            main_children: Vec::new(),
            next_id: 1,
            main: Rect {
                x: 0,
                y: 0,
                width: columns,
                height: rows,
            },
            status: None,
            layout_changed: false,
            changed: BTreeSet::new(),
        }
    }

    /// The screen has a new size.
    pub fn resize(&mut self, rows: usize, columns: usize) {
        if (rows, columns) != (self.rows, self.columns) {
            self.rows = rows;
            self.columns = columns;
            self.lay_out();
        }
    }

    /// The screen's size, as lines and columns.
    pub fn size(&self) -> (usize, usize) {
        (self.rows, self.columns)
    }

    /// What is left for the main window.
    pub fn main(&self) -> Rect {
        self.main
    }

    /// Create a pane, or `None` if it cannot be.
    pub fn create(
        &mut self,
        parent: Option<u32>,
        placement: Placement,
        spec: PaneSpec,
    ) -> Option<u32> {
        if spec.kind != kind::TEXT && spec.kind != kind::GRID {
            return None;
        }
        // A parent that is not a pane means the main window.
        let parent = parent.filter(|id| self.windows.contains_key(id));
        if parent.is_none()
            && !self.side_panes
            && (spec.align == align::LEFT || spec.align == align::RIGHT)
        {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        // A pane starts with no size in the dimension it can be sized
        // in, and the main window's in the other.
        let rect = if spec.align == align::LEFT || spec.align == align::RIGHT {
            Rect {
                height: self.main.height,
                ..Rect::default()
            }
        } else {
            Rect {
                width: self.main.width,
                ..Rect::default()
            }
        };
        let mut flags =
            spec.style & (style::AUTO_VSCROLL | style::MORE_MODE | style::HSTRUT | style::VSTRUT);
        if flags & style::MORE_MODE != 0 {
            flags |= style::AUTO_VSCROLL;
        }
        self.windows.insert(
            id,
            Window {
                parent,
                attached: true,
                children: Vec::new(),
                kind: spec.kind,
                align: spec.align,
                size: spec.size,
                units: spec.units,
                flags,
                lines: Vec::new(),
                x: 0,
                y: 0,
                xmax: 0,
                ymax: 0,
                rect,
                screen_color: None,
                fitted: false,
            },
        );
        let list = self.children_mut(parent);
        let at = match placement {
            Placement::First => 0,
            Placement::Last => list.len(),
            Placement::Before(other) => list.iter().position(|&c| c == other).unwrap_or(list.len()),
            Placement::After(other) => list
                .iter()
                .position(|&c| c == other)
                .map_or(list.len(), |at| at + 1),
        };
        list.insert(at, id);
        if spec.size != 0 {
            self.lay_out();
        }
        Some(id)
    }

    /// Delete a pane. Its own panes stay, but are no longer shown.
    pub fn delete(&mut self, id: u32) {
        let Some(window) = self.windows.remove(&id) else {
            return;
        };
        if window.attached {
            self.children_mut(window.parent).retain(|&c| c != id);
        }
        for child in window.children {
            if let Some(child) = self.windows.get_mut(&child) {
                child.parent = None;
                child.attached = false;
            }
        }
        if self.status == Some(id) {
            self.status = None;
        }
        self.changed.remove(&id);
        self.lay_out();
    }

    /// Remove a pane's text.
    pub fn clear(&mut self, id: u32) {
        if let Some(window) = self.windows.get_mut(&id) {
            window.clear();
            self.changed.insert(id);
        }
    }

    /// Add text to a pane. A newline goes to the start of the next line.
    pub fn write(&mut self, id: u32, text: &str) {
        let Some(window) = self.windows.get_mut(&id) else {
            return;
        };
        for c in text.chars() {
            match c {
                '\n' => window.newline(),
                '\r' => window.x = 0,
                _ => window.put(c),
            }
        }
        self.changed.insert(id);
    }

    /// Move where the next character goes in a grid, counting from zero.
    /// Text panes have no positions, and ignore this.
    pub fn go_to(&mut self, id: u32, row: usize, column: usize) {
        if let Some(window) = self.windows.get_mut(&id) {
            if window.kind == kind::GRID {
                window.y = row;
                window.x = column;
            }
        }
    }

    /// Change a pane's size. An advisory size is only a guess for hosts
    /// that cannot size a pane to its contents, so it is ignored.
    pub fn set_size(&mut self, id: u32, size: i32, units: i32, advisory: bool) {
        let Some(window) = self.windows.get_mut(&id) else {
            return;
        };
        if advisory {
            return;
        }
        window.fitted = false;
        if (window.size, window.units) == (size, units) {
            return;
        }
        window.size = size;
        window.units = units;
        self.lay_out();
    }

    /// Make a pane just big enough for what it holds, in the dimension
    /// it can be sized in.
    pub fn size_to_contents(&mut self, id: u32) {
        let Some(window) = self.windows.get_mut(&id) else {
            return;
        };
        window.fitted = true;
        let window = &self.windows[&id];
        let (size, current) = if window.horizontal() {
            (self.content_height(id), window.rect.height)
        } else {
            (self.content_width(id), window.rect.width)
        };
        if size == current {
            return;
        }
        if let Some(window) = self.windows.get_mut(&id) {
            window.size = i32::try_from(size).unwrap_or(i32::MAX);
            window.units = units::CELLS;
        }
        self.lay_out();
    }

    fn content_height(&self, id: u32) -> usize {
        let window = &self.windows[&id];
        let mut height = window.ymax + 1;
        for &child in &window.children {
            let chi = &self.windows[&child];
            if chi.flags & style::VSTRUT != 0 {
                let child_height = self.content_height(child);
                if chi.horizontal() {
                    height += child_height;
                } else {
                    height = height.max(child_height);
                }
            }
        }
        height
    }

    fn content_width(&self, id: u32) -> usize {
        let window = &self.windows[&id];
        let mut width = window.xmax + 1;
        for &child in &window.children {
            let chi = &self.windows[&child];
            if chi.flags & style::HSTRUT != 0 {
                let child_width = self.content_width(child);
                if chi.horizontal() {
                    width = width.max(child_width);
                } else {
                    width += child_width;
                }
            }
        }
        width
    }

    /// Give a pane a background color.
    pub fn set_screen_color(&mut self, id: u32, color: u32) {
        if let Some(window) = self.windows.get_mut(&id) {
            if window.screen_color != Some(color) {
                window.screen_color = Some(color);
                self.changed.insert(id);
            }
        }
    }

    /// The background color a pane was given, if any. The old-style
    /// status line has the status line's.
    pub fn screen_color(&self, id: u32) -> Option<u32> {
        if self.status == Some(id) {
            return Some(color::STATUS_BG);
        }
        self.windows.get(&id).and_then(|window| window.screen_color)
    }

    /// What a program can learn about a pane.
    pub fn info(&self, id: u32) -> Option<PaneInfo> {
        let window = self.windows.get(&id)?;
        Some(PaneInfo {
            align: window.align,
            // A fixed-pitch display lines text up at tab stops.
            style: window.flags | style::TAB_ALIGN,
            rows: window.rect.height,
            columns: window.rect.width,
        })
    }

    /// How a pane is placed, as the program asked.
    pub fn layout(&self, id: u32) -> Option<PaneLayout> {
        let window = self.windows.get(&id)?;
        Some(PaneLayout {
            parent: window.parent,
            kind: window.kind,
            align: window.align,
            size: window.size,
            units: window.units,
            fitted: window.fitted,
        })
    }

    /// Whether a pane can be at the left or right of the main window: a
    /// display that can keep the main window's text to part of its width
    /// says yes.
    pub fn set_side_panes(&mut self, side_panes: bool) {
        self.side_panes = side_panes;
    }

    /// Show the old-style status line, the program's location at the left
    /// and its score at the right, in a line of its own at the top.
    pub fn set_status(&mut self, left: &str, right: &str) {
        let id = match self.status {
            Some(id) => id,
            None => {
                let spec = PaneSpec {
                    kind: kind::GRID,
                    align: align::TOP,
                    size: 1,
                    units: units::CELLS,
                    style: 0,
                };
                let Some(id) = self.create(None, Placement::First, spec) else {
                    return;
                };
                self.status = Some(id);
                id
            }
        };
        let width = self.windows[&id].rect.width;
        let left: Vec<char> = left.chars().collect();
        let right: Vec<char> = right.chars().collect();
        let mut line = vec![' '; width];
        for (i, &c) in left.iter().take(width.saturating_sub(1)).enumerate() {
            line[i + 1] = c;
        }
        let start = width.saturating_sub(right.len() + 1);
        for (i, &c) in right.iter().enumerate() {
            if let Some(cell) = line.get_mut(start + i) {
                *cell = c;
            }
        }
        if let Some(window) = self.windows.get_mut(&id) {
            window.clear();
            window.lines.push(line);
            self.changed.insert(id);
        }
    }

    /// The old-style status line, if there is one.
    pub fn status(&self) -> Option<u32> {
        self.status
    }

    /// The panes on the screen, outermost first, with where each is.
    pub fn panes(&self) -> Vec<(u32, Rect)> {
        let mut out = Vec::new();
        self.collect(&self.main_children, &mut out);
        out
    }

    fn collect(&self, list: &[u32], out: &mut Vec<(u32, Rect)>) {
        for &id in list {
            let window = &self.windows[&id];
            out.push((id, window.rect));
            self.collect(&window.children, out);
        }
    }

    /// The area a pane shows its own text in: what is left once its own
    /// panes have taken their space.
    pub fn rect(&self, id: u32) -> Option<Rect> {
        self.windows.get(&id).map(|window| window.rect)
    }

    /// What a pane shows, a line for each line of its area, each as wide
    /// as the area.
    pub fn view(&self, id: u32) -> Vec<String> {
        let Some(window) = self.windows.get(&id) else {
            return Vec::new();
        };
        let Rect { width, height, .. } = window.rect;
        // A text pane that keeps its last line in view shows the end of
        // its text; anything else shows the start.
        let last = window.lines.len().min(window.y + 1);
        let first = if window.kind == kind::TEXT && window.flags & style::AUTO_VSCROLL != 0 {
            last.saturating_sub(height)
        } else {
            0
        };
        (first..first + height)
            .map(|row| {
                let mut line: String = window
                    .lines
                    .get(row)
                    .map(|chars| chars.iter().take(width).collect())
                    .unwrap_or_default();
                let len = line.chars().count();
                line.extend(std::iter::repeat_n(' ', width - len));
                line
            })
            .collect()
    }

    /// Whether the layout has changed since this was last asked, and
    /// everything needs drawing again.
    pub fn take_layout_changed(&mut self) -> bool {
        std::mem::take(&mut self.layout_changed)
    }

    /// The panes whose text has changed since this was last asked.
    pub fn take_changed(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.changed).into_iter().collect()
    }

    fn children_mut(&mut self, parent: Option<u32>) -> &mut Vec<u32> {
        match parent {
            Some(id) => &mut self.windows.get_mut(&id).expect("parent pane").children,
            None => &mut self.main_children,
        }
    }

    /// Work out where everything goes: the main window starts with the
    /// whole screen, and each pane in turn takes its space from what its
    /// parent has left.
    fn lay_out(&mut self) {
        let mut area = Rect {
            x: 0,
            y: 0,
            width: self.columns,
            height: self.rows,
        };
        let children = self.main_children.clone();
        for id in children {
            self.lay_out_window(id, &mut area);
        }
        self.main = area;
        self.layout_changed = true;
    }

    fn lay_out_window(&mut self, id: u32, parent: &mut Rect) {
        let window = &self.windows[&id];
        let base = if window.horizontal() {
            parent.height
        } else {
            parent.width
        };
        let size = match window.units {
            units::PERCENT => usize::try_from(window.size).unwrap_or(0) * base / 100,
            _ => usize::try_from(window.size).unwrap_or(0),
        };
        let mut rect = *parent;
        match window.align {
            align::TOP => {
                rect.height = size.min(parent.height);
                parent.y += rect.height;
                parent.height -= rect.height;
            }
            align::BOTTOM => {
                rect.height = size.min(parent.height);
                rect.y = parent.y + parent.height - rect.height;
                parent.height -= rect.height;
            }
            align::LEFT => {
                rect.width = size.min(parent.width);
                parent.x += rect.width;
                parent.width -= rect.width;
            }
            _ => {
                rect.width = size.min(parent.width);
                rect.x = parent.x + parent.width - rect.width;
                parent.width -= rect.width;
            }
        }
        let children = window.children.clone();
        for child in children {
            self.lay_out_window(child, &mut rect);
        }
        self.windows.get_mut(&id).expect("pane").rect = rect;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(align: i32, size: i32, units: i32) -> PaneSpec {
        PaneSpec {
            kind: kind::TEXT,
            align,
            size,
            units,
            style: 0,
        }
    }

    #[test]
    fn panes_take_their_space_in_order() {
        let mut screen = Screen::new(24, 80, true);
        let top = screen
            .create(None, Placement::Last, spec(align::TOP, 2, units::CELLS))
            .unwrap();
        let bottom = screen
            .create(
                None,
                Placement::Last,
                spec(align::BOTTOM, 25, units::PERCENT),
            )
            .unwrap();
        let left = screen
            .create(None, Placement::Last, spec(align::LEFT, 10, units::CELLS))
            .unwrap();
        assert_eq!(
            screen.rect(top).unwrap(),
            Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 2
            }
        );
        // A quarter of the 22 lines the top pane left.
        assert_eq!(
            screen.rect(bottom).unwrap(),
            Rect {
                x: 0,
                y: 19,
                width: 80,
                height: 5
            }
        );
        assert_eq!(
            screen.rect(left).unwrap(),
            Rect {
                x: 0,
                y: 2,
                width: 10,
                height: 17
            }
        );
        assert_eq!(
            screen.main(),
            Rect {
                x: 10,
                y: 2,
                width: 70,
                height: 17
            }
        );

        // Placed first, a pane takes its space before the others.
        let first = screen
            .create(None, Placement::First, spec(align::TOP, 1, units::CELLS))
            .unwrap();
        assert_eq!(screen.rect(first).unwrap().y, 0);
        assert_eq!(screen.rect(top).unwrap().y, 1);
    }

    #[test]
    fn a_pane_within_a_pane_takes_from_its_parent() {
        let mut screen = Screen::new(24, 80, true);
        let outer = screen
            .create(None, Placement::Last, spec(align::TOP, 6, units::CELLS))
            .unwrap();
        let inner = screen
            .create(
                Some(outer),
                Placement::Last,
                spec(align::RIGHT, 20, units::CELLS),
            )
            .unwrap();
        assert_eq!(
            screen.rect(inner).unwrap(),
            Rect {
                x: 60,
                y: 0,
                width: 20,
                height: 6
            }
        );
        assert_eq!(
            screen.rect(outer).unwrap(),
            Rect {
                x: 0,
                y: 0,
                width: 60,
                height: 6
            }
        );
        screen.delete(outer);
        assert_eq!(screen.panes(), []);
        assert_eq!(screen.main().height, 24);
    }

    #[test]
    fn a_panes_layout_is_as_the_program_asked() {
        let mut screen = Screen::new(24, 80, false);
        screen.set_side_panes(true);
        let side = screen
            .create(None, Placement::Last, spec(align::LEFT, 30, units::PERCENT))
            .unwrap();
        let layout = screen.layout(side).unwrap();
        assert_eq!(
            (
                layout.parent,
                layout.align,
                layout.size,
                layout.units,
                layout.fitted
            ),
            (None, align::LEFT, 30, units::PERCENT, false)
        );
        screen.size_to_contents(side);
        assert!(screen.layout(side).unwrap().fitted);
        // An advisory size leaves it fitted; a size it is given does not.
        screen.set_size(side, 10, units::CELLS, true);
        assert!(screen.layout(side).unwrap().fitted);
        screen.set_size(side, 10, units::CELLS, false);
        assert!(!screen.layout(side).unwrap().fitted);
        assert_eq!(screen.layout(side + 1), None);
    }

    #[test]
    fn a_terminal_has_no_panes_at_the_sides() {
        let mut screen = Screen::new(24, 80, false);
        assert_eq!(
            screen.create(None, Placement::Last, spec(align::LEFT, 10, units::CELLS)),
            None
        );
        let top = screen
            .create(None, Placement::Last, spec(align::TOP, 3, units::CELLS))
            .unwrap();
        assert!(
            screen
                .create(
                    Some(top),
                    Placement::Last,
                    spec(align::LEFT, 10, units::CELLS)
                )
                .is_some()
        );
    }

    #[test]
    fn sizing_to_contents_counts_the_lines_ended() {
        let mut screen = Screen::new(24, 80, true);
        let id = screen
            .create(None, Placement::Last, spec(align::TOP, 0, units::CELLS))
            .unwrap();
        assert_eq!(screen.info(id).unwrap().rows, 0);
        screen.write(id, "Kitchen          Score: 5\n");
        screen.size_to_contents(id);
        assert_eq!(screen.info(id).unwrap().rows, 1);
        screen.write(id, "Exits: north\n");
        screen.size_to_contents(id);
        assert_eq!(
            screen.view(id),
            [
                format!("{:<80}", "Kitchen          Score: 5"),
                format!("{:<80}", "Exits: north")
            ]
        );
        // Advisory sizes are for hosts that cannot do this.
        screen.set_size(id, 5, units::CELLS, true);
        assert_eq!(screen.info(id).unwrap().rows, 2);
        screen.clear(id);
        screen.size_to_contents(id);
        assert_eq!(screen.info(id).unwrap().rows, 1);
    }

    #[test]
    fn a_grid_is_written_at_a_position() {
        let mut screen = Screen::new(24, 20, true);
        let id = screen
            .create(
                None,
                Placement::Last,
                PaneSpec {
                    kind: kind::GRID,
                    align: align::TOP,
                    size: 2,
                    units: units::CELLS,
                    style: 0,
                },
            )
            .unwrap();
        screen.go_to(id, 1, 3);
        screen.write(id, "abc");
        screen.go_to(id, 1, 4);
        screen.write(id, "X");
        assert_eq!(
            screen.view(id),
            [" ".repeat(20), format!("{:<20}", "   aXc")]
        );
    }

    #[test]
    fn a_scrolling_pane_shows_its_last_lines() {
        let mut screen = Screen::new(24, 10, true);
        let id = screen
            .create(
                None,
                Placement::Last,
                PaneSpec {
                    style: style::AUTO_VSCROLL,
                    ..spec(align::TOP, 2, units::CELLS)
                },
            )
            .unwrap();
        screen.write(id, "one\ntwo\nthree");
        assert_eq!(
            screen.view(id),
            [format!("{:<10}", "two"), format!("{:<10}", "three")]
        );
        assert_eq!(
            screen.info(id).unwrap().style,
            style::AUTO_VSCROLL | style::TAB_ALIGN
        );
    }

    #[test]
    fn the_status_line_is_a_line_at_the_top() {
        let mut screen = Screen::new(24, 30, false);
        screen.set_status("Kitchen", "5/12");
        let id = screen.status().unwrap();
        assert_eq!(screen.view(id), [format!(" {:<24}5/12 ", "Kitchen")]);
        assert_eq!(screen.main().y, 1);
    }
}
