//! The Date and TimeZone built-in classes.
//!
//! A Date is a moment in time, kept as a day number and a time of day in
//! UTC. The day number counts from March 1 of the year 0 in the Gregorian
//! calendar run back before it began, which puts the leap day at the end
//! of the counting year and makes the calendar arithmetic simple.

use std::rc::Rc;

use crate::decimal::{self, Big};
use crate::error::{VmError, VmResult, code};
use crate::meta::MetaKind;
use crate::object::Object;
use crate::tz::{LocalZone, Zone, ZoneCache, normalize};
use crate::value::{NativeDesc, ObjId, Value};
use crate::vm::Vm;

const MS_PER_DAY: i32 = 24 * 60 * 60 * 1000;
const DAY_MS: f64 = 24.0 * 60.0 * 60.0 * 1000.0;

/// The day number of January 1, 1970.
pub(crate) const UNIX_EPOCH_DAYNO: i32 = 719_468;

/// Division that rounds toward negative infinity.
fn divfl(a: i64, b: i64) -> i64 {
    a.div_euclid(b)
}

/// A date on the Gregorian calendar. The month and day need not be in
/// range: day 0 is the last day of the month before, and month 13 is
/// January of the next year.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CalDate {
    pub y: i32,
    pub m: i32,
    pub d: i32,
}

impl CalDate {
    pub(crate) fn new(y: i32, m: i32, d: i32) -> Self {
        Self { y, m, d }
    }

    pub(crate) fn from_dayno(dayno: i32) -> Self {
        let mut date = Self::new(0, 3, 1);
        date.set_dayno(dayno);
        date
    }

    pub(crate) fn set_dayno(&mut self, dayno: i32) {
        let dayno = i64::from(dayno);
        let days_before = |y: i64| 365 * y + divfl(y, 4) - divfl(y, 100) + divfl(y, 400);
        let mut y = ((10000.0 * dayno as f64 + 14780.0) / 3_652_425.0).floor() as i64;
        let mut d = dayno - days_before(y);
        if d < 0 {
            y -= 1;
            d = dayno - days_before(y);
        }
        let m = (100 * d + 52) / 3060;
        self.y = (y + (m + 2) / 12) as i32;
        self.m = ((m + 2) % 12 + 1) as i32;
        self.d = (d - (m * 306 + 5) / 10 + 1) as i32;
    }

    pub(crate) fn dayno(&self) -> i32 {
        // Count the year from March.
        let months = i64::from(self.m) - 3;
        let m = months.rem_euclid(12);
        let y = i64::from(self.y) + months.div_euclid(12);
        (365 * y + divfl(y, 4) - divfl(y, 100)
            + divfl(y, 400)
            + (m * 306 + 5) / 10
            + (i64::from(self.d) - 1)) as i32
    }

    /// Bring the month and day into range.
    pub(crate) fn normalize(&mut self) {
        self.set_dayno(self.dayno());
    }

    /// The day of the week, from 0 for Sunday, however far out of range
    /// the month and day are.
    pub(crate) fn weekday_by_dayno(&self) -> i32 {
        (self.dayno() + 3).rem_euclid(7)
    }

    /// The day of the week, from 0 for Sunday. This is the reference
    /// VM's formula, which goes negative for years before 1.
    pub(crate) fn weekday(&self) -> i32 {
        const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        if !(1..=12).contains(&self.m) {
            return self.weekday_by_dayno();
        }
        let yy = self.y - i32::from(self.m < 3);
        (yy + yy / 4 - yy / 100 + yy / 400 + T[self.m as usize - 1] + self.d) % 7
    }

    /// The day of the week by ISO 8601: 1 for Monday through 7.
    fn iso_weekday(&self) -> i32 {
        match self.weekday() {
            0 => 7,
            w => w,
        }
    }

    /// The ISO 8601 week number, and the year the week belongs to.
    fn iso_weekno(&self) -> (i32, i32) {
        // The week belongs to the year its Thursday is in.
        let this_thursday = self.dayno() - (self.iso_weekday() - 1) + 3;
        let year = Self::from_dayno(this_thursday).y;
        let jan1 = Self::new(year, 1, 1);
        let first_thursday = jan1.dayno() + (11 - jan1.weekday()) % 7;
        ((this_thursday - first_thursday) / 7 + 1, year)
    }

    fn julian_dayno(&self) -> f64 {
        f64::from(self.dayno()) + 1_721_119.5
    }

    /// The same day on the Julian calendar.
    fn julian_date(&self) -> (i32, i32, i32) {
        let z = (self.julian_dayno() + 0.5).floor();
        let b = z + 1524.0;
        let c = ((b - 122.1) / 365.25).floor();
        let d = (365.25 * c).floor();
        let e = ((b - d) / 30.6001).floor();
        let month = if e < 14.0 { e - 1.0 } else { e - 13.0 };
        let year = if month > 2.0 { c - 4716.0 } else { c - 4715.0 };
        let day = b - d - (30.6001 * e).floor();
        (year as i32, month as i32, day as i32)
    }

    /// Set the date from one on the Julian calendar.
    fn set_julian_date(&mut self, jy: i32, jm: i32, jd: i32) {
        // Bring the month into range, as the reference VM does, which
        // leaves January and February as months 13 and 14.
        let months = jm - 1;
        let (mut quot, mut rem) = (months / 12, months % 12);
        if rem <= 0 {
            rem += 12;
            quot -= 1;
        }
        let mut jm = rem + 1;
        let mut jy = jy + quot;
        if jm <= 2 {
            jy -= 1;
            jm += 12;
        }
        let dayno = (365.25 * (f64::from(jy) + 4716.0)).floor()
            + (30.6001 * (f64::from(jm) + 1.0)).floor()
            + f64::from(jd)
            - 1524.5
            - 1_721_119.5;
        self.set_dayno(dayno as i32);
    }
}

/// A date on either calendar.
#[derive(Debug, Clone, Copy)]
struct Calendar {
    julian: bool,
    date: CalDate,
    y: i32,
    m: i32,
    d: i32,
}

impl Calendar {
    fn new(julian: bool) -> Self {
        Self {
            julian,
            date: CalDate::new(0, 3, 1),
            y: 0,
            m: 3,
            d: 1,
        }
    }

    fn set_dayno(&mut self, dayno: i32) {
        self.date.set_dayno(dayno);
        (self.y, self.m, self.d) = if self.julian {
            self.date.julian_date()
        } else {
            (self.date.y, self.date.m, self.date.d)
        };
    }

    fn set_date(&mut self, y: i32, m: i32, d: i32) {
        if self.julian {
            self.date.set_julian_date(y, m, d);
        } else {
            self.date = CalDate::new(y, m, d);
        }
        (self.y, self.m, self.d) = (y, m, d);
    }

    fn dayno(&self) -> i32 {
        self.date.dayno()
    }

    fn jan1_dayno(&self) -> i32 {
        let mut jan1 = CalDate::new(self.y, 1, 1);
        if self.julian {
            jan1.set_julian_date(self.y, 1, 1);
        }
        jan1.dayno()
    }

    /// The week of the year, where weeks start on a given weekday and
    /// week 1 starts on the first such day.
    fn weekno(&self, weekday: i32) -> i32 {
        let jan1 = CalDate::from_dayno(self.jan1_dayno());
        let delta = (7 + weekday - jan1.weekday()) % 7;
        let week0 = jan1.dayno() + delta - 7;
        (self.dayno() - week0) / 7
    }
}

/// The settings that say how dates are written and read, by number.
const LOCALE_DEFAULTS: [&str; 15] = [
    "January,February,March,April,May,June,July,August,September,October,November,December",
    "Jan,Feb,Mar,Apr,May,Jun,Jul,Aug,Sep=Sept,Oct,Nov,Dec",
    "Sunday,Monday,Tuesday,Wednesday,Thursday,Friday,Saturday",
    "Sun,Mon,Tue,Wed,Thu,Fri,Sat",
    "AM=A.M.,PM=P.M.,am,pm",
    "AD=A.D.=CE=C.E.,BC=B.C.=BCE=B.C.E.",
    "us",
    "st,nd,rd,th,st,nd,rd",
    "%a %b %#d %T %Y",
    "%H:%M:%S",
    "%m/%d/%Y",
    "%m/%d/%y",
    "%#I:%M:%S %P",
    "%H:%M",
    "%H:%M:%S",
];
const LC_MONTH: usize = 0;
const LC_MON: usize = 1;
const LC_WEEKDAY: usize = 2;
const LC_WKDY: usize = 3;
const LC_AMPM: usize = 4;
const LC_ERA: usize = 5;
const LC_PARSE_FILTER: usize = 6;
const LC_ORDSUF: usize = 7;
const LC_FMT_TIMESTAMP: usize = 8;
const LC_FMT_TIME: usize = 9;
const LC_FMT_DATE: usize = 10;
const LC_FMT_SHORTDATE: usize = 11;
const LC_FMT_12HOUR: usize = 12;
const LC_FMT_24HOUR: usize = 13;
const LC_FMT_24HOURSECS: usize = 14;

/// The locale settings in force: the program's where it has set them,
/// and the defaults elsewhere.
struct Locale(Vec<Vec<u8>>);

impl Locale {
    fn get(&self, item: usize) -> &[u8] {
        self.0.get(item).map_or(&[], Vec::as_slice)
    }

    /// An element of a setting that is a list separated by commas. An
    /// element can give several spellings separated by `=`, of which the
    /// first is the one to write. With `sticky`, an index past the end is
    /// the last element.
    fn index_list(&self, item: usize, index: i32, sticky: bool) -> &[u8] {
        let mut rest = self.get(item);
        let mut index = index;
        while index > 0 && !rest.is_empty() {
            match rest.iter().position(|&b| b == b',') {
                Some(comma) => rest = &rest[comma + 1..],
                None if sticky => break,
                None => return &[],
            }
            index -= 1;
        }
        let end = rest
            .iter()
            .position(|&b| b == b'=' || b == b',')
            .unwrap_or(rest.len());
        &rest[..end]
    }
}

/// Which ordinal suffix a number takes: `st`, `nd`, `rd`, or `th`.
fn ordinal_index(n: i32) -> i32 {
    let last = n % 10;
    if last == 0 || last >= 4 || (n > 10 && n < 20) {
        3
    } else {
        last + if n < 10 { -1 } else { 3 }
    }
}

/// Round to the nearest integer, halves away from zero.
fn round_i32(value: f64) -> i32 {
    value.round() as i32
}

/// A part of the text a date was parsed from.
#[derive(Debug, Clone, Copy, Default)]
struct Capture {
    start: Option<usize>,
    len: usize,
}

const UNSET: i32 = i32::MIN;

/// What has been made out of a date string so far.
#[derive(Debug, Clone)]
struct Parsed {
    /// 1 or -1 for a sign before the year, 2 or -2 for AD or BC.
    era: i32,
    yy: i32,
    yy_needs_century: bool,
    mm: i32,
    dd: i32,
    /// An ISO week and weekday, as seven times the week plus the day.
    week: i32,
    /// 1 for AM, 2 for PM.
    ampm: i32,
    hh: i32,
    mi: i32,
    ss: i32,
    ms: i32,
    zone: Option<Rc<Zone>>,
    /// An offset from UTC in milliseconds that the text gave outright.
    tzofs: i32,
    captures: [Capture; 13],
}

// The captures, in the order `parseDate` reports them.
const CAP_ERA: usize = 0;
const CAP_YY: usize = 1;
const CAP_MM: usize = 2;
const CAP_DD: usize = 3;
const CAP_DOY: usize = 4;
const CAP_WEEK: usize = 5;
const CAP_AMPM: usize = 6;
const CAP_HH: usize = 7;
const CAP_MI: usize = 8;
const CAP_SS: usize = 9;
const CAP_MS: usize = 10;
const CAP_UNIX: usize = 11;
const CAP_TZ: usize = 12;

impl Parsed {
    fn new() -> Self {
        Self {
            era: 0,
            yy: UNSET,
            yy_needs_century: false,
            mm: UNSET,
            dd: UNSET,
            week: UNSET,
            ampm: 0,
            hh: UNSET,
            mi: UNSET,
            ss: UNSET,
            ms: UNSET,
            zone: None,
            tzofs: UNSET,
            captures: [Capture::default(); 13],
        }
    }

    fn default_to(field: &mut i32, value: i32) {
        if *field == UNSET {
            *field = value;
        }
    }

    fn local_to_utc(&self, dayno: &mut i32, daytime: &mut i32) {
        if self.tzofs != UNSET {
            *daytime = daytime.wrapping_sub(self.tzofs);
            normalize(dayno, daytime);
        } else if let Some(zone) = &self.zone {
            zone.local_to_utc(dayno, daytime);
        }
    }

    fn utc_to_local(&self, dayno: &mut i32, daytime: &mut i32) {
        if self.tzofs != UNSET {
            *daytime = daytime.wrapping_add(self.tzofs);
            normalize(dayno, daytime);
        } else if let Some(zone) = &self.zone {
            zone.utc_to_local(dayno, daytime);
        }
    }

    /// Fill in the parts of the date the text left out from a reference
    /// date, and work out the year from the era.
    fn default_date(&mut self, calendar: &mut Calendar, dayno: i32) {
        calendar.set_dayno(dayno);
        if self.era != 0 && self.yy != UNSET {
            match self.era {
                2 => self.yy_needs_century = false,
                -1 => self.yy = self.yy.wrapping_neg(),
                -2 => {
                    self.yy = self.yy.wrapping_neg().wrapping_add(1);
                    self.yy_needs_century = false;
                }
                _ => {}
            }
        }
        Self::default_to(&mut self.yy, calendar.y);
        Self::default_to(&mut self.mm, calendar.m);
        Self::default_to(&mut self.dd, calendar.d);
        // A two-digit year is the one within fifty years of now.
        if self.yy_needs_century {
            self.yy += (calendar.y / 100) * 100;
            if self.yy < calendar.y - 50 {
                self.yy += 100;
            }
            if self.yy > calendar.y + 49 {
                self.yy -= 100;
            }
        }
    }

    fn default_time(&mut self) {
        if self.ampm == 1 && self.hh == 12 {
            self.hh = 0;
        } else if self.ampm == 2 && self.hh < 12 {
            self.hh += 12;
        }
        Self::default_to(&mut self.hh, 0);
        Self::default_to(&mut self.mi, 0);
        Self::default_to(&mut self.ss, 0);
        Self::default_to(&mut self.ms, 0);
    }

    fn dayno(&self, calendar: &mut Calendar) -> i32 {
        if self.week != UNSET && self.yy != UNSET {
            // Week 1 is the week January 4 is in.
            let correction = CalDate::new(self.yy, 1, 4).iso_weekday() + 3;
            CalDate::new(self.yy, 1, self.week - correction).dayno()
        } else {
            calendar.set_date(self.yy, self.mm, self.dd);
            calendar.dayno()
        }
    }

    fn daytime(&self) -> i32 {
        self.hh
            .wrapping_mul(60 * 60 * 1000)
            .wrapping_add(self.mi.wrapping_mul(60 * 1000))
            .wrapping_add(self.ss.wrapping_mul(1000))
            .wrapping_add(self.ms)
    }
}

/// A date string once parsed: the moment in UTC, what was made out of
/// the text, and the formats that matched it.
struct ParsedDate {
    dayno: i32,
    daytime: i32,
    parsed: Parsed,
    formats: Vec<Vec<u8>>,
}

/// The date formats `new Date` and `parseDate` know without being told.
/// A format is words separated by spaces. Most words name a part of a
/// date; any other is a set of characters, one of which has to appear,
/// with `_` for a space, and `*`, `?`, or `+` at the end saying how many
/// times. `[us]` and `[eu]` mark formats for one locale only.
const BUILTIN_FORMATS: [&str; 55] = [
    "[us]m / d / y",
    "[us]m / d",
    "[us]m - d",
    "[us]m . d",
    "[us]m .\t- d .\t- y",
    "[eu]d / m / y",
    "[eu]d / m",
    "[eu]d - m",
    "[eu]d . m",
    "[eu]d .\t- m .\t- y",
    "yyyy mm dd",
    "yyyy .? doy",
    "m / y",
    "m - y",
    "month _.\t-* ddth _,.\t+ ye",
    "ddth _.\t-* month _,.\t-* ye",
    "month _.\t-* ddth",
    "ddth _.\t-* month",
    "month - dd - ye",
    "month _\t.-* y",
    "month",
    "+- y - mm - dd",
    "y / m / d",
    "y - m - d",
    "y / m",
    "y - m",
    "ye _\t.-* month _\t.-* d",
    "ye _\t.-* month",
    "ye - month - dd",
    "yyyy",
    "era _* y",
    "y _* era",
    "h _? ampm",
    "h : mi",
    "h .: mi _? ampm",
    "h .: mi .: ss _? ampm",
    "h : mi : ss",
    "h : mi : ss .: ssfrac _? ampm",
    "h : mi : ss .: ssfrac",
    "tT? hh .: mi",
    "tT? hh mi",
    "tT? hh .: mi .: ss",
    "tT? hh mi SS",
    "tT? hh .: mi .: ss _? tz",
    "tT? hh .: mi .: ss . ssfrac",
    "tz",
    "d / mon / yyyy : hh : mi : ss _ gmtofs",
    "yyyy : mm : dd _ hh : mi : ss",
    "yyyy -? \\W W",
    "yyyy - mm - dd \\T hh : mi : ss . ssfrac",
    "yyyy - mm - dd \\T hh : mi : ss . ssfrac gmtofs",
    "@ unix",
    "yyyy mm dd \\T hh : mi : ss",
    "yyyy mm dd \\t hh mi ss",
    "yyyy - m - d \\T h : i : s",
];

fn is_c_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// Match digits against a template: for each digit, the lowest and the
/// highest it can be. A comma in the template is a place where the text
/// may have a comma or a period, as in `1,000`. Returns the number and
/// the position after it.
fn match_digits(text: &[u8], pos: usize, template: &[u8]) -> Option<(i32, usize)> {
    let mut at = pos;
    let mut acc: i32 = 0;
    let mut t = 0;
    while t < template.len() {
        if template[t] == b',' {
            if matches!(text.get(at), Some(b',' | b'.')) {
                at += 1;
            }
            t += 1;
            continue;
        }
        let digit = *text.get(at)?;
        if !digit.is_ascii_digit() || digit < template[t] || digit > *template.get(t + 1)? {
            return None;
        }
        acc = acc.wrapping_mul(10).wrapping_add(i32::from(digit - b'0'));
        at += 1;
        t += 2;
    }
    Some((acc, at))
}

/// The character before a position and the one at it.
fn chars_around(text: &[u8], pos: usize) -> (Option<char>, Option<char>) {
    let before = String::from_utf8_lossy(&text[..pos]).chars().next_back();
    let after = String::from_utf8_lossy(&text[pos..]).chars().next();
    (before, after)
}

/// Match literal text, ignoring case unless the literal starts with `^`.
/// A match cannot end in the middle of a word or of a number. Returns
/// the position after the match.
fn match_lit(text: &[u8], pos: usize, literal: &[u8]) -> Option<usize> {
    let (literal, fold) = match literal.split_first() {
        Some((b'^', rest)) => (rest, false),
        _ => (literal, true),
    };
    let rest = &text[pos..];
    let matched = if fold {
        fold_prefix_len(literal, rest)?
    } else if rest.starts_with(literal) {
        literal.len()
    } else {
        return None;
    };
    let end = pos + matched;
    if end < text.len() && !literal.is_empty() {
        if let (Some(last), Some(next)) = chars_around(text, end) {
            if (last.is_ascii_alphabetic() && next.is_ascii_alphabetic())
                || (last.is_ascii_digit() && next.is_ascii_digit())
            {
                return None;
            }
        }
    }
    Some(end)
}

/// If text starts with a literal when case is ignored, how many bytes of
/// the text the literal accounts for.
fn fold_prefix_len(literal: &[u8], text: &[u8]) -> Option<usize> {
    let fold = |c: char, out: &mut Vec<char>| match crate::casefold::fold(c) {
        Some(folded) => out.extend(folded.chars()),
        None => out.push(c),
    };
    let mut wanted = Vec::new();
    for c in String::from_utf8_lossy(literal).chars() {
        fold(c, &mut wanted);
    }
    let text = String::from_utf8_lossy(text);
    let mut found = Vec::new();
    let mut used = 0;
    for c in text.chars() {
        if found.len() >= wanted.len() {
            break;
        }
        fold(c, &mut found);
        used += c.len_utf8();
    }
    (found == wanted).then_some(used)
}

/// Match one of the elements of a list as a locale setting gives it:
/// elements separated by commas, each with alternative spellings
/// separated by `=`, up to a semicolon. Returns the number of the element,
/// from 1, whose spelling matched the most text, and the position after.
fn match_list(text: &[u8], pos: usize, list: &[u8]) -> Option<(i32, usize)> {
    let mut best: Option<(i32, usize)> = None;
    let mut rest = list;
    while let [first, tail @ ..] = rest {
        if !is_c_space(*first) {
            break;
        }
        rest = tail;
    }
    let mut index = 1;
    while !rest.is_empty() {
        let end = rest
            .iter()
            .position(|&b| matches!(b, b'=' | b',' | b';'))
            .unwrap_or(rest.len());
        if end != 0 {
            if let Some(after) = match_lit(text, pos, &rest[..end]) {
                if after - pos > best.map_or(0, |(_, best_end)| best_end - pos) {
                    best = Some((index, after));
                }
            }
        }
        match rest.get(end) {
            None | Some(b';') => break,
            Some(b',') => {
                index += 1;
                rest = &rest[end + 1..];
                while let [first, tail @ ..] = rest {
                    if !is_c_space(*first) {
                        break;
                    }
                    rest = tail;
                }
            }
            _ => rest = &rest[end + 1..],
        }
    }
    best
}

const DATE_METHODS: [NativeDesc; 14] = [
    NativeDesc::fixed(0),
    NativeDesc::optional(1, 3), // parseDate
    NativeDesc::optional(1, 3), // parseJulianDate
    NativeDesc::optional(1, 1), // formatDate
    NativeDesc::optional(1, 1), // formatJulianDate
    NativeDesc::fixed(1),       // compareTo
    NativeDesc::optional(0, 1), // getDate
    NativeDesc::fixed(0),       // getJulianDay
    NativeDesc::optional(0, 1), // getJulianDate
    NativeDesc::optional(0, 1), // getISOWeekDate
    NativeDesc::optional(0, 1), // getClockTime
    NativeDesc::fixed(1),       // addInterval
    NativeDesc::optional(2, 1), // findWeekday
    NativeDesc::variable(0),    // setLocaleInfo
];

impl Vm {
    // ---- zones ----

    /// Use the zone cache, with a way for it to ask the host about the
    /// local zone if it turns out to need to know.
    fn with_zones<R>(
        &mut self,
        work: impl FnOnce(&mut ZoneCache, &mut dyn FnMut() -> LocalZone) -> R,
    ) -> R {
        let host = &mut self.host;
        let mut local = || LocalZone {
            name: host.local_time_zone(),
            utc_offset_seconds: host.time().utc_offset_seconds,
        };
        work(&mut self.io.zones, &mut local)
    }

    pub(crate) fn local_zone(&mut self) -> Rc<Zone> {
        self.with_zones(|zones, local| zones.local_zone(local))
    }

    /// The moment the host's clock shows.
    pub(crate) fn now(&mut self) -> (i32, i32) {
        let millis = self.host.time().unix_millis;
        let dayno = millis.div_euclid(i64::from(MS_PER_DAY)) + i64::from(UNIX_EPOCH_DAYNO);
        (
            dayno as i32,
            millis.rem_euclid(i64::from(MS_PER_DAY)) as i32,
        )
    }

    fn is_numeric(&self, value: Value) -> bool {
        matches!(value, Value::Int(_)) || self.as_big(value).is_some()
    }

    fn num_to_f64(&mut self, value: Value) -> VmResult<f64> {
        match value {
            Value::Int(n) => Ok(n.into()),
            _ => match self.as_big(value) {
                Some(big) => Ok(big.to_text().parse().unwrap_or(0.0)),
                None => Err(VmError::new(code::NUM_VAL_REQD)),
            },
        }
    }

    /// The zone a constructor's arguments name: none for the local zone,
    /// a name, or an offset in seconds.
    fn zone_from_args(&mut self, first: usize, argc: usize) -> VmResult<Rc<Zone>> {
        if argc == 0 {
            return Ok(self.local_zone());
        }
        if argc > 1 {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        let arg = self.arg(first);
        let zone = if let Some(name) = self.as_str(arg) {
            let name = String::from_utf8_lossy(&name).into_owned();
            self.with_zones(|zones, local| zones.parse_zone(&name, local))
        } else if self.is_numeric(arg) {
            let seconds = self.cast_to_int(arg)?;
            Some(self.io.zones.offset_zone(seconds))
        } else {
            return Err(VmError::new(code::BAD_TYPE_BIF));
        };
        zone.ok_or_else(|| VmError::new(code::BAD_VAL_BIF))
    }

    /// The time zone argument of a Date method: missing or nil for the
    /// local zone, a TimeZone, or what a TimeZone can be made from.
    fn zone_arg(&mut self, index: usize, argc: u32) -> VmResult<Rc<Zone>> {
        if index as u32 >= argc || self.arg(index) == Value::Nil {
            return Ok(self.local_zone());
        }
        if let Value::Obj(id) = self.arg(index) {
            if let Some(Object::TimeZone(zone)) = self.objects.get(id) {
                return Ok(zone.clone());
            }
        }
        self.zone_from_args(index, argc as usize - index)
    }

    /// `new TimeZone(...)`.
    pub(crate) fn create_time_zone(&mut self, argc: u32) -> VmResult<ObjId> {
        let zone = self.zone_from_args(0, argc as usize)?;
        Ok(self.alloc_obj(Object::TimeZone(zone)))
    }

    /// Look up the zones of time zone objects that were read from an
    /// image or a saved state. A name the VM does not know keeps the
    /// offsets and abbreviation recorded with it.
    pub(crate) fn resolve_zones(&mut self) {
        for id in self.objects.pending_zones() {
            let Some(Object::TimeZone(zone)) = self.objects.get(id) else {
                continue;
            };
            let name = zone.name().to_owned();
            let Some((gmtofs, save, abbr)) = zone.pending.clone() else {
                continue;
            };
            let found = self.with_zones(|zones, local| zones.parse_zone(&name, local));
            let zone = found.unwrap_or_else(|| {
                let desc = crate::tz::ZoneDesc::recorded(gmtofs, save, &abbr);
                self.io.zones.missing_zone(&name, &desc)
            });
            self.objects.replace(id, Object::TimeZone(zone));
        }
    }

    /// Note the time, for the zones in a state about to be saved.
    pub(crate) fn note_save_time(&mut self) {
        self.io.save_time = self.now();
    }

    /// A zone's saved form: its offsets and abbreviation at the time
    /// noted, and its name.
    pub(crate) fn zone_saved_form(&self, zone: &Rc<Zone>) -> (i32, i32, String, String) {
        let (dayno, daytime) = self.io.save_time;
        let query = zone.query(dayno, daytime, false);
        let name = if self.io.zones.is_local(zone) {
            ":local".to_owned()
        } else {
            zone.name().to_owned()
        };
        (query.gmtofs, query.save, query.abbr.to_string(), name)
    }

    pub(crate) fn zone_name(&self, id: ObjId) -> Option<String> {
        match self.objects.get(id) {
            Some(Object::TimeZone(zone)) => Some(zone.name().to_owned()),
            _ => None,
        }
    }

    /// Put the elements into a list created ahead of them, as the
    /// reference VM creates it.
    fn set_list(&mut self, list: Value, items: Vec<Value>) {
        if let Some(Object::List(stored)) = self.objects.get_mut(list.obj_id()) {
            *stored = items.into();
        }
    }

    /// One entry of a zone's history: when a period began, the offsets
    /// during it, and its abbreviation.
    fn history_item(
        &mut self,
        start: Option<(i32, i32)>,
        gmtofs: i32,
        save: i32,
        abbr: &str,
    ) -> Value {
        let list = self.new_list(vec![Value::Nil; 4]);
        let date = match start {
            Some((dayno, daytime)) => self.new_date(dayno, daytime),
            None => Value::Nil,
        };
        let abbr = self.new_str(abbr);
        self.set_list(list, vec![date, Value::Int(gmtofs), Value::Int(save), abbr]);
        list
    }

    pub(crate) fn time_zone_method(
        &mut self,
        id: ObjId,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let desc = match index {
            1 | 3 | 4 => NativeDesc::fixed(0),
            2 => NativeDesc::optional(0, 1),
            _ => return Ok(None),
        };
        self.native_method(argc, desc, |vm, argc| vm.time_zone_call(id, index, argc))
            .map(Some)
    }

    fn time_zone_call(&mut self, id: ObjId, index: u16, argc: u32) -> VmResult<Value> {
        let Some(Object::TimeZone(zone)) = self.objects.get(id) else {
            return Ok(Value::Nil);
        };
        let zone = zone.clone();
        Ok(match index {
            // getNames()
            1 => {
                let names = zone.names();
                let list = self.new_list(vec![Value::Nil; names.len()]);
                let items = names.iter().map(|name| self.new_str(name)).collect();
                self.set_list(list, items);
                list
            }
            // getHistory(date?)
            2 => {
                if argc >= 1 && self.arg(0) != Value::Nil {
                    let Some((dayno, daytime)) = self.as_date(self.arg(0)) else {
                        return Err(VmError::new(code::BAD_TYPE_BIF));
                    };
                    let query = zone.query(dayno, daytime, false);
                    let start = (query.start_dayno != i32::MIN || query.start_daytime != i32::MIN)
                        .then_some((query.start_dayno, query.start_daytime));
                    self.history_item(start, query.gmtofs, query.save, &query.abbr)
                } else {
                    let transitions = zone.transitions().to_vec();
                    let list = self.new_list(vec![Value::Nil; transitions.len() + 1]);
                    let first = zone.time_type(0).clone();
                    let mut items =
                        vec![self.history_item(None, first.gmtofs, first.save, &first.abbr)];
                    for transition in &transitions {
                        let time_type = zone.time_type(transition.time_type).clone();
                        items.push(self.history_item(
                            Some((transition.dayno, transition.daytime)),
                            time_type.gmtofs,
                            time_type.save,
                            &time_type.abbr,
                        ));
                    }
                    self.set_list(list, items);
                    list
                }
            }
            // getRules()
            3 => {
                let rules = zone.rules().to_vec();
                let list = self.new_list(vec![Value::Nil; rules.len()]);
                let mut items = Vec::new();
                for rule in &rules {
                    let entry = self.new_list(vec![Value::Nil; 10]);
                    const MONTHS: [&str; 12] = [
                        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct",
                        "Nov", "Dec",
                    ];
                    const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
                    let described = if rule.when == 0 && rule.day > 31 {
                        format!("DOY {}", rule.day)
                    } else if rule.when < 4 && (1..=7).contains(&rule.weekday) {
                        let month = MONTHS
                            .get((rule.month - 1) as usize)
                            .copied()
                            .unwrap_or_default();
                        let weekday = WEEKDAYS[(rule.weekday - 1) as usize];
                        match rule.when {
                            0 => format!("{month} {:02}", rule.day),
                            1 => format!("{month} last {weekday}"),
                            2 => format!("{month} {weekday}>={}", rule.day),
                            // The reference VM's template for this one
                            // leaves the day as the letter d.
                            _ => format!("{month} {weekday}<=d"),
                        }
                    } else {
                        format!(
                            "INVAL(when={}, weekday={}, dd={})",
                            rule.when, rule.weekday, rule.day
                        )
                    };
                    let abbr = self.new_str(&rule.abbr);
                    let described = self.new_str(&described);
                    let clock = self.new_str(match rule.at_zone {
                        0x80 => "u",
                        0x40 => "s",
                        _ => "w",
                    });
                    self.set_list(
                        entry,
                        vec![
                            abbr,
                            Value::Int(rule.gmtofs),
                            Value::Int(rule.save),
                            described,
                            Value::Int(rule.month),
                            Value::Int(rule.when.into()),
                            Value::Int(rule.day),
                            Value::Int(rule.weekday),
                            Value::Int(rule.at),
                            clock,
                        ],
                    );
                    items.push(entry);
                }
                self.set_list(list, items);
                list
            }
            // getLocation(): the country, the latitude, the longitude,
            // and a comment.
            _ => {
                let list = self.new_list(vec![Value::Nil; 4]);
                let coords = zone.coords.as_str();
                let split = coords
                    .char_indices()
                    .skip(1)
                    .find(|&(_, c)| c == '+' || c == '-')
                    .map_or(coords.len(), |(at, _)| at);
                let country = self.new_str(&zone.country);
                let latitude = self.new_str(&coords[..split]);
                let longitude = self.new_str(&coords[split..]);
                let comment = match &zone.desc {
                    Some(desc) if !desc.is_empty() => self.new_str(desc),
                    _ => Value::Nil,
                };
                self.set_list(list, vec![country, latitude, longitude, comment]);
                list
            }
        })
    }

    // ---- dates ----

    pub(crate) fn new_date(&mut self, dayno: i32, daytime: i32) -> Value {
        Value::Obj(self.alloc_obj(Object::Date { dayno, daytime }))
    }

    /// A Date for a count of seconds since the start of 1970.
    pub(crate) fn date_from_unix(&mut self, seconds: i64) -> Value {
        let dayno = seconds.div_euclid(86400) + i64::from(UNIX_EPOCH_DAYNO);
        let daytime = seconds.rem_euclid(86400) * 1000;
        self.new_date(dayno as i32, daytime as i32)
    }

    pub(crate) fn as_date(&self, value: Value) -> Option<(i32, i32)> {
        match value {
            Value::Obj(id) => match self.objects.get(id)? {
                Object::Date { dayno, daytime } => Some((*dayno, *daytime)),
                _ => None,
            },
            _ => None,
        }
    }

    /// The locale settings: the program's, where it has made any.
    fn locale(&self) -> Locale {
        let class = self.meta.class_obj(MetaKind::Date);
        let state = match self.objects.get(class) {
            Some(Object::Class(class)) => class.class_state,
            _ => Value::Nil,
        };
        let set: &[Value] = match state {
            Value::Obj(id) => match self.objects.get(id) {
                Some(Object::Vector(items)) => items,
                _ => &[],
            },
            _ => &[],
        };
        Locale(
            (0..LOCALE_DEFAULTS.len().max(set.len()))
                .map(|index| {
                    set.get(index)
                        .and_then(|&value| self.as_str(value))
                        .map(|text| text.to_vec())
                        .or_else(|| {
                            LOCALE_DEFAULTS
                                .get(index)
                                .map(|text| text.as_bytes().to_vec())
                        })
                        .unwrap_or_default()
                })
                .collect(),
        )
    }

    /// Change a locale setting.
    fn set_locale(&mut self, index: i32, value: Value) -> VmResult<()> {
        let class = self.meta.class_obj(MetaKind::Date);
        let state = match self.objects.get(class) {
            Some(Object::Class(class)) => class.class_state,
            _ => return Ok(()),
        };
        // The settings are kept in a vector that the class object holds,
        // so that they are saved, restored, and undone like anything else.
        let vector = match state {
            Value::Obj(id) if matches!(self.objects.get(id), Some(Object::Vector(_))) => id,
            _ => {
                let id = self.alloc_obj(Object::Vector(vec![Value::Nil; LOCALE_DEFAULTS.len()]));
                if let Some(Object::Class(class)) = self.objects.get_mut(class) {
                    class.class_state = Value::Obj(id);
                }
                id
            }
        };
        if !(0..LOCALE_DEFAULTS.len() as i32).contains(&index) {
            return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
        }
        if let Some(Object::Vector(items)) = self.objects.get_mut(vector) {
            if let Some(slot) = items.get_mut(index as usize) {
                *slot = value;
            }
        }
        Ok(())
    }

    /// Match an offset from UTC: `+0500`, `-3:30`, `GMT+1`.
    fn match_tz_offset(&mut self, parsed: &mut Parsed, text: &[u8], pos: usize) -> Option<usize> {
        let mut at = pos;
        let gmt = match match_lit(text, at, b"^GMT") {
            Some(after) => {
                at = after;
                true
            }
            None => false,
        };
        let sign = match text.get(at) {
            Some(b'+') => 1,
            Some(b'-') => -1,
            _ => return None,
        };
        at += 1;
        let (hh, after) = match_digits(text, at, b"0509")
            .or_else(|| gmt.then(|| match_digits(text, at, b"09")).flatten())?;
        at = after;
        // Minutes and seconds, each after a colon or run together.
        let part = |at: &mut usize| -> Option<i32> {
            if text.get(*at) == Some(&b':') {
                let (value, after) = match_digits(text, *at + 1, b"0509")?;
                *at = after;
                Some(value)
            } else {
                Some(match match_digits(text, *at, b"0509") {
                    Some((value, after)) => {
                        *at = after;
                        value
                    }
                    None => 0,
                })
            }
        };
        let mi = part(&mut at)?;
        let ss = part(&mut at)?;
        let seconds = sign * (hh * 3600 + mi * 60 + ss);
        parsed.zone = Some(self.io.zones.offset_zone(seconds));
        parsed.tzofs = seconds * 1000;
        parsed.captures[CAP_TZ] = Capture {
            start: Some(pos),
            len: at - pos,
        };
        Some(at)
    }

    /// Match a zone by name: one from the database, such as
    /// `America/New_York`, or an abbreviation, such as `EST`.
    fn match_tz_name(&mut self, parsed: &mut Parsed, text: &[u8], pos: usize) -> Option<usize> {
        if !text.get(pos)?.is_ascii_alphabetic() {
            return None;
        }
        let mut end = pos;
        while let Some(&c) = text.get(end) {
            let joiner =
                (c == b'_' || c == b'/') && text.get(end + 1).is_some_and(u8::is_ascii_alphabetic);
            if !c.is_ascii_alphabetic() && !joiner {
                break;
            }
            end += 1;
        }
        let name = String::from_utf8_lossy(&text[pos..end]).into_owned();
        let capture = Capture {
            start: Some(pos),
            len: end - pos,
        };
        if name.contains('/') {
            if let Some(zone) = self.io.zones.database_zone(&name) {
                parsed.zone = Some(zone);
                parsed.captures[CAP_TZ] = capture;
                return Some(end);
            }
        }
        let now = self.now();
        let (zone, offset) =
            self.with_zones(|zones, local| zones.zone_by_abbr(&name, now, local))?;
        parsed.zone = Some(zone);
        if let Some(offset) = offset {
            parsed.tzofs = offset;
        }
        parsed.captures[CAP_TZ] = capture;
        Some(end)
    }

    /// Match text against one date format. Returns the position after
    /// what matched, or `None` if the text does not fit the format.
    fn parse_with_format(
        &mut self,
        parsed: &mut Parsed,
        calendar: &mut Calendar,
        text: &[u8],
        start: usize,
        format: &[u8],
        locale: &Locale,
    ) -> VmResult<Option<usize>> {
        let mut pos = start;
        while text
            .get(pos)
            .is_some_and(|&c| crate::regex::is_space(c.into()))
        {
            pos += 1;
        }
        let mut fmt = format;
        // Match digits against the first template that fits.
        let digits = |pos: usize, templates: &[&[u8]]| {
            templates
                .iter()
                .find_map(|template| match_digits(text, pos, template))
        };
        let capture = |from: usize, to: usize| Capture {
            start: Some(from),
            len: to - from,
        };
        const YEAR: [&[u8]; 5] = [
            b"09,090909,090909",
            b"090909,090909",
            b"0909,090909",
            b"09090909",
            b"090909",
        ];
        while pos < text.len() {
            while let [b' ', rest @ ..] = fmt {
                fmt = rest;
            }
            if fmt.is_empty() {
                break;
            }
            let word_len = fmt.iter().position(|&b| b == b' ').unwrap_or(fmt.len());
            let word = &fmt[..word_len];
            fmt = &fmt[word_len..];
            // A number for a field, from the templates given.
            macro_rules! number {
                ($cap:expr, $($template:expr),+) => {
                    match digits(pos, &[$($template),+]) {
                        Some((value, after)) => {
                            parsed.captures[$cap] = capture(pos, after);
                            pos = after;
                            value
                        }
                        None => return Ok(None),
                    }
                };
            }
            match word {
                b"d" => parsed.dd = number!(CAP_DD, b"0209", b"3301", b"09"),
                b"dd" => parsed.dd = number!(CAP_DD, b"0009", b"1209", b"3301"),
                b"ddth" => {
                    let day = number!(CAP_DD, b"0209", b"3301", b"09");
                    parsed.dd = day;
                    let suffix = locale.index_list(LC_ORDSUF, ordinal_index(day), true);
                    if let Some(after) = match_lit(text, pos, suffix) {
                        parsed.captures[CAP_DD].len += after - pos;
                        pos = after;
                    }
                }
                b"doy" => {
                    let templates: [&[u8]; 5] =
                        [b"000019", b"001909", b"120909", b"330509", b"336606"];
                    if let Some((value, after)) = digits(pos, &templates) {
                        parsed.captures[CAP_DOY] = capture(pos, after);
                        pos = after;
                        parsed.mm = 1;
                        parsed.dd = value;
                    }
                }
                b"month" | b"mon" => {
                    const ROMAN: &[u8] = b"^I,^II,^III,^IV,^V,^VI,^VII,^VIII,^IX,^X,^XI,^XII;";
                    let found = if word == b"month" {
                        match_list(text, pos, locale.get(LC_MONTH))
                            .or_else(|| match_list(text, pos, locale.get(LC_MON)))
                            .or_else(|| match_list(text, pos, ROMAN))
                    } else {
                        match_list(text, pos, locale.get(LC_MON))
                    };
                    let Some((month, after)) = found else {
                        return Ok(None);
                    };
                    parsed.captures[CAP_MM] = capture(pos, after);
                    pos = after;
                    parsed.mm = month;
                    Parsed::default_to(&mut parsed.dd, 1);
                }
                b"m" => {
                    parsed.mm = number!(CAP_MM, b"0009", b"1102", b"09");
                    Parsed::default_to(&mut parsed.dd, 1);
                }
                b"mm" => {
                    parsed.mm = number!(CAP_MM, b"0009", b"1102");
                    Parsed::default_to(&mut parsed.dd, 1);
                }
                b"mi" => {
                    parsed.mi = number!(CAP_MI, b"0509");
                    Parsed::default_to(&mut parsed.ss, 0);
                    Parsed::default_to(&mut parsed.ms, 0);
                }
                b"W" => {
                    // An ISO week, and optionally a weekday. The
                    // reference VM takes the format to be finished here,
                    // whatever follows in it.
                    let week = number!(CAP_WEEK, b"0019", b"1409", b"5503");
                    let weekday = |at: usize| {
                        text.get(at)
                            .filter(|c| (b'1'..=b'7').contains(c))
                            .map(|c| i32::from(c - b'0'))
                    };
                    let mut day = 1;
                    if let (Some(b'-'), Some(found)) = (text.get(pos), weekday(pos + 1)) {
                        day = found;
                        pos += 2;
                        parsed.captures[CAP_WEEK].len += 2;
                    } else if let Some(found) = weekday(pos) {
                        day = found;
                        pos += 1;
                        parsed.captures[CAP_WEEK].len += 1;
                    }
                    parsed.week = week * 7 + day;
                    return Ok(Some(pos));
                }
                b"y" => {
                    if let Some((value, after)) = digits(pos, &YEAR) {
                        parsed.captures[CAP_YY] = capture(pos, after);
                        pos = after;
                        parsed.yy = value;
                    } else {
                        parsed.yy = number!(CAP_YY, b"0909", b"09");
                        parsed.yy_needs_century = true;
                    }
                    Parsed::default_to(&mut parsed.mm, 1);
                    Parsed::default_to(&mut parsed.dd, 1);
                }
                b"yy" => {
                    parsed.yy = number!(CAP_YY, b"0909");
                    parsed.yy_needs_century = true;
                    Parsed::default_to(&mut parsed.mm, 1);
                    Parsed::default_to(&mut parsed.dd, 1);
                }
                b"yyyy" => {
                    parsed.yy = number!(CAP_YY, b"09090909");
                    Parsed::default_to(&mut parsed.mm, 1);
                    Parsed::default_to(&mut parsed.dd, 1);
                }
                b"ye" => {
                    // A year with an era before or after it.
                    let eras = locale.get(LC_ERA);
                    let mut era = 0;
                    if let Some((index, after)) = match_list(text, pos, eras) {
                        era = if index == 1 { 2 } else { -2 };
                        parsed.era = era;
                        parsed.captures[CAP_ERA] = capture(pos, after);
                        pos = after;
                        while text.get(pos).is_some_and(|&c| is_c_space(c)) {
                            pos += 1;
                        }
                    }
                    let Some((value, after)) =
                        digits(pos, &YEAR).or_else(|| digits(pos, &[b"0909", b"09"]))
                    else {
                        return Ok(None);
                    };
                    parsed.captures[CAP_YY] = capture(pos, after);
                    pos = after;
                    parsed.yy = value;
                    if !(-5_879_650 < value && value < 5_879_650) {
                        return Err(VmError::new(code::NUM_OVERFLOW));
                    }
                    if era == 0 {
                        let mut at = pos;
                        while text.get(at).is_some_and(|&c| is_c_space(c)) {
                            at += 1;
                        }
                        if let Some((index, after)) = match_list(text, at, eras) {
                            era = if index == 1 { 2 } else { -2 };
                            parsed.era = era;
                            parsed.captures[CAP_ERA] = capture(at, after);
                            pos = after;
                        }
                    }
                    if era == 0 && parsed.captures[CAP_YY].len <= 2 {
                        parsed.yy_needs_century = true;
                    }
                }
                b"era" => {
                    let Some((index, after)) = match_list(text, pos, locale.get(LC_ERA)) else {
                        return Ok(None);
                    };
                    parsed.era = if index == 1 { 2 } else { -2 };
                    parsed.captures[CAP_ERA] = capture(pos, after);
                    pos = after;
                }
                b"h" | b"hh" => {
                    parsed.hh = if word == b"h" {
                        number!(CAP_HH, b"0019", b"1102", b"09")
                    } else {
                        number!(CAP_HH, b"0109", b"2204")
                    };
                    Parsed::default_to(&mut parsed.mi, 0);
                    Parsed::default_to(&mut parsed.ss, 0);
                    Parsed::default_to(&mut parsed.ms, 0);
                }
                b"i" => {
                    parsed.mi = number!(CAP_MI, b"0509", b"09");
                    Parsed::default_to(&mut parsed.ss, 0);
                    Parsed::default_to(&mut parsed.ms, 0);
                }
                b"ss" => {
                    parsed.ss = number!(CAP_SS, b"0509");
                    Parsed::default_to(&mut parsed.ms, 0);
                }
                b"s" => {
                    parsed.ss = number!(CAP_SS, b"0509", b"09");
                    Parsed::default_to(&mut parsed.ms, 0);
                }
                b"ssfrac" => {
                    // A fraction of a second, rounded to milliseconds.
                    // The reference VM looks for a digit one character
                    // further on than the fraction starts.
                    if !text.get(pos + 1).is_some_and(u8::is_ascii_digit) {
                        return Ok(None);
                    }
                    let from = pos;
                    let (mut acc, mut scale, mut trailing) = (0, 10000, false);
                    while let Some(&c) = text.get(pos).filter(|c| c.is_ascii_digit()) {
                        if scale > 1 {
                            acc = acc * 10 + i32::from(c - b'0');
                        } else if c != b'0' {
                            trailing = true;
                        }
                        pos += 1;
                        scale /= 10;
                    }
                    if scale != 0 {
                        acc *= scale;
                    }
                    let last = acc % 10;
                    acc /= 10;
                    if last > 5 || (last == 5 && (trailing || (acc % 10) & 1 != 0)) {
                        acc += 1;
                    }
                    parsed.ms = acc;
                    parsed.captures[CAP_MS] = capture(from, pos);
                }
                b"unix" => {
                    // A count of seconds from the start of 1970.
                    let from = pos;
                    let mut sign = 1.0;
                    let digit_at = |at: usize| text.get(at).is_some_and(u8::is_ascii_digit);
                    if text.get(pos) == Some(&b'-') && digit_at(pos + 1) {
                        sign = -1.0;
                        pos += 1;
                    } else if !digit_at(pos) {
                        return Ok(None);
                    }
                    let mut acc = 0.0;
                    while digit_at(pos) {
                        acc = acc * 10.0 + f64::from(text[pos] - b'0');
                        pos += 1;
                    }
                    acc *= sign;
                    let days = (acc / 86400.0).floor();
                    let seconds = (acc % 86400.0) as i32;
                    if days > f64::from(i32::MAX) {
                        return Ok(None);
                    }
                    calendar.set_dayno((days as i32).wrapping_add(UNIX_EPOCH_DAYNO));
                    (parsed.yy, parsed.mm, parsed.dd) = (calendar.y, calendar.m, calendar.d);
                    parsed.hh = seconds / 3600;
                    parsed.mi = (seconds / 60) % 60;
                    parsed.ss = seconds % 60;
                    parsed.ms = 0;
                    parsed.tzofs = 0;
                    parsed.captures[CAP_UNIX] = capture(from, pos);
                }
                b"ampm" => {
                    let Some((index, after)) = match_list(text, pos, locale.get(LC_AMPM)) else {
                        return Ok(None);
                    };
                    parsed.ampm = index;
                    parsed.captures[CAP_AMPM] = capture(pos, after);
                    pos = after;
                }
                b"tz" => {
                    let found = match self.match_tz_offset(parsed, text, pos) {
                        Some(after) => Some(after),
                        None => self.match_tz_name(parsed, text, pos),
                    };
                    match found {
                        Some(after) => pos = after,
                        None => return Ok(None),
                    }
                }
                b"gmtofs" => match self.match_tz_offset(parsed, text, pos) {
                    Some(after) => pos = after,
                    None => return Ok(None),
                },
                b"+-" => {
                    let (era, after) =
                        match (match_lit(text, pos, b"+"), match_lit(text, pos, b"-")) {
                            (Some(after), _) => (1, after),
                            (None, Some(after)) => (-1, after),
                            _ => return Ok(None),
                        };
                    parsed.era = era;
                    parsed.captures[CAP_ERA] = capture(pos, after);
                    pos = after;
                }
                _ => {
                    // Literal characters: any one of the set, as many
                    // times as the suffix allows.
                    let suffix = word[word.len() - 1];
                    let min = usize::from(!matches!(suffix, b'*' | b'?'));
                    let max = if matches!(suffix, b'*' | b'+') {
                        65535
                    } else {
                        1
                    };
                    let set = if matches!(suffix, b'*' | b'?' | b'+') {
                        &word[..word.len() - 1]
                    } else {
                        word
                    };
                    let set: Vec<char> = String::from_utf8_lossy(set).chars().collect();
                    let mut allowed = Vec::new();
                    let mut index = 0;
                    while index < set.len() {
                        let mut c = set[index];
                        if c == '\\' && index + 1 < set.len() {
                            index += 1;
                            c = set[index];
                        } else if c == '_' {
                            c = ' ';
                        }
                        allowed.push(c);
                        index += 1;
                    }
                    let mut count = 0;
                    while count < max && pos < text.len() {
                        let Some(next) = String::from_utf8_lossy(&text[pos..]).chars().next()
                        else {
                            break;
                        };
                        if !allowed.contains(&next) {
                            break;
                        }
                        count += 1;
                        // A byte that is not valid text counts as one.
                        pos += if std::str::from_utf8(
                            &text[pos..pos + next.len_utf8().min(text.len() - pos)],
                        )
                        .is_ok()
                        {
                            next.len_utf8()
                        } else {
                            1
                        };
                    }
                    if count < min {
                        return Ok(None);
                    }
                }
            }
        }
        // All of the format has to be used.
        if !fmt.is_empty() {
            return Ok(None);
        }
        while text
            .get(pos)
            .is_some_and(|&c| crate::regex::is_space(c.into()))
        {
            pos += 1;
        }
        Ok(Some(pos))
    }

    /// Parse a date string: at each point in it, take whichever format
    /// matches the most text. Returns the date in UTC, what was made out,
    /// and the formats used.
    fn parse_date_string(
        &mut self,
        text: &[u8],
        custom: Value,
        julian: bool,
        reference: (i32, i32),
        zone: &Rc<Zone>,
    ) -> VmResult<Option<ParsedDate>> {
        let locale = self.locale();
        // The formats to try: the program's own, with nil among them
        // standing for the built-in ones, or just the built-in ones.
        let mut formats: Vec<Vec<u8>> = Vec::new();
        let builtin = |formats: &mut Vec<Vec<u8>>| {
            formats.extend(
                BUILTIN_FORMATS
                    .iter()
                    .map(|format| format.as_bytes().to_vec()),
            );
        };
        if custom == Value::Nil {
            builtin(&mut formats);
        } else if let Some(format) = self.as_str(custom) {
            formats.push(format.to_vec());
        } else if let Some(len) = self.list_like_len(custom)? {
            for index in 1..=len {
                let element = self.list_like_get(custom, index)?;
                if element == Value::Nil {
                    builtin(&mut formats);
                } else if let Some(format) = self.as_str(element) {
                    formats.push(format.to_vec());
                } else {
                    // Anything else ends the list.
                    break;
                }
            }
        } else {
            return Err(VmError::new(code::BAD_TYPE_BIF));
        }
        let filter = locale.get(LC_PARSE_FILTER).to_vec();

        let mut calendar = Calendar::new(julian);
        let mut result = Parsed::new();
        let mut used = Vec::new();
        let mut pos = 0;
        while pos < text.len() {
            let mut best: Option<(usize, Parsed, &Vec<u8>)> = None;
            for format in &formats {
                // A format for another locale is passed over.
                let mut body = format.as_slice();
                if body.len() >= 3 && body[0] == b'[' {
                    if let Some(close) = body.iter().position(|&b| b == b']') {
                        if !body[1..close].eq_ignore_ascii_case(&filter) {
                            continue;
                        }
                        body = &body[close + 1..];
                    }
                }
                let mut attempt = result.clone();
                let matched =
                    self.parse_with_format(&mut attempt, &mut calendar, text, pos, body, &locale)?;
                if let Some(end) = matched {
                    if end - pos > best.as_ref().map_or(0, |(best_end, _, _)| best_end - pos) {
                        best = Some((end, attempt, format));
                    }
                }
            }
            let Some((end, parsed, format)) = best else {
                return Ok(None);
            };
            if used.len() < 20 {
                used.push(format.clone());
            }
            pos = end;
            result = parsed;
            while text.get(pos).is_some_and(|c| b" \t;,:".contains(c)) {
                pos += 1;
            }
        }

        // Fill in what the text left out from the reference date, as the
        // zone the date is in sees it.
        if result.zone.is_none() {
            result.zone = Some(zone.clone());
        }
        let (mut ref_day, mut ref_time) = reference;
        result.utc_to_local(&mut ref_day, &mut ref_time);
        result.default_date(&mut calendar, ref_day);
        result.default_time();
        let mut dayno = result.dayno(&mut calendar);
        let mut daytime = result.daytime();
        normalize(&mut dayno, &mut daytime);
        result.local_to_utc(&mut dayno, &mut daytime);
        Ok(Some(ParsedDate {
            dayno,
            daytime,
            parsed: result,
            formats: used,
        }))
    }

    /// A date from year, month, and day arguments, and optionally hours,
    /// minutes, seconds, and milliseconds, as a day and time not yet
    /// adjusted for any zone.
    fn date_from_parts(&mut self, argc: usize) -> VmResult<(i32, i32)> {
        let mut parts = [0i32; 7];
        for (index, part) in parts.iter_mut().enumerate().take(argc) {
            *part = self.cast_to_int(self.arg(index))?;
        }
        let mut dayno = CalDate::new(parts[0], parts[1], parts[2]).dayno();
        let mut daytime = 0;
        if argc == 7 {
            let ms = f64::from(parts[3]) * 3_600_000.0
                + f64::from(parts[4]) * 60_000.0
                + f64::from(parts[5]) * 1000.0
                + f64::from(parts[6]);
            let days = (ms / DAY_MS).floor() + f64::from(dayno);
            if days < f64::from(i32::MIN) || days > f64::from(i32::MAX) {
                return Err(VmError::new(code::NUM_OVERFLOW));
            }
            dayno = days as i32;
            daytime = (ms % DAY_MS) as i32;
        }
        normalize(&mut dayno, &mut daytime);
        Ok((dayno, daytime))
    }

    /// `new Date(...)`.
    pub(crate) fn create_date(&mut self, argc: u32) -> VmResult<ObjId> {
        let numeric =
            |vm: &Self, count: usize| (0..count).all(|index| vm.is_numeric(vm.arg(index)));
        let first_text = if argc >= 1 {
            self.as_str(self.arg(0)).map(|text| text.to_vec())
        } else {
            None
        };
        let (dayno, daytime) = if argc == 0 {
            self.now()
        } else if let (1..=3, Some(text)) = (argc, &first_text) {
            // A date string, a zone to read it in, and a date to take
            // missing parts from.
            let zone = if argc >= 2 && self.arg(1) != Value::Nil {
                self.zone_arg(1, argc)?
            } else {
                self.local_zone()
            };
            let reference = if argc >= 3 && self.arg(2) != Value::Nil {
                self.as_date(self.arg(2))
                    .ok_or_else(|| VmError::new(code::BAD_TYPE_BIF))?
            } else {
                self.now()
            };
            match self.parse_date_string(text, Value::Nil, false, reference, &zone)? {
                Some(found) => (found.dayno, found.daytime),
                None => return Err(VmError::new(code::BAD_VAL_BIF)),
            }
        } else if argc == 2 && self.is_numeric(self.arg(0)) && self.as_str(self.arg(1)).is_some() {
            // A number and what it counts: 'U' for seconds from the
            // start of 1970, 'J' for a Julian day number.
            let kind = self.as_str(self.arg(1)).map(|text| text.to_vec());
            let value = self.num_to_f64(self.arg(0))?;
            let (days, ms) = match kind.as_deref() {
                Some(b"U" | b"u") => {
                    let days = (value / 86400.0).floor();
                    (
                        days + f64::from(UNIX_EPOCH_DAYNO),
                        (value - days * 86400.0) * 1000.0,
                    )
                }
                Some(b"J" | b"j") => {
                    let value = value - 1_721_119.5;
                    let days = value.floor();
                    (days, (value - days) * DAY_MS)
                }
                _ => return Err(VmError::new(code::BAD_VAL_BIF)),
            };
            if days > f64::from(i32::MAX) || days < f64::from(i32::MIN) {
                return Err(VmError::new(code::NUM_OVERFLOW));
            }
            (days as i32, round_i32(ms))
        } else if (argc == 3 || argc == 7) && numeric(self, argc as usize) {
            let (mut dayno, mut daytime) = self.date_from_parts(argc as usize)?;
            self.local_zone().local_to_utc(&mut dayno, &mut daytime);
            (dayno, daytime)
        } else if (argc == 4 || argc == 8) && numeric(self, argc as usize - 1) {
            let (mut dayno, mut daytime) = self.date_from_parts(argc as usize - 1)?;
            self.zone_arg(argc as usize - 1, argc)?
                .local_to_utc(&mut dayno, &mut daytime);
            (dayno, daytime)
        } else {
            return Err(VmError::new(code::BAD_TYPE_BIF));
        };
        Ok(self.alloc_obj(Object::Date { dayno, daytime }))
    }

    /// A count of days from a number, for adding to a date: whole days,
    /// and the rest in milliseconds.
    fn day_count(&mut self, value: Value) -> VmResult<(f64, i32)> {
        let days = self.num_to_f64(value)?;
        let whole = days.floor();
        Ok((whole, round_i32((days - whole) * DAY_MS)))
    }

    /// A date and a number of days, for `+`.
    pub(crate) fn date_add(&mut self, id: ObjId, other: Value) -> VmResult<Value> {
        let (mut dayno, mut daytime) = self.as_date(Value::Obj(id)).unwrap_or_default();
        if let Value::Int(days) = other {
            dayno = dayno.wrapping_add(days);
        } else {
            let (whole, ms) = self.day_count(other)?;
            let sum = whole + f64::from(dayno);
            if sum > f64::from(i32::MAX) || sum < f64::from(i32::MIN) {
                return Err(VmError::new(code::NUM_OVERFLOW));
            }
            dayno = sum as i32;
            daytime += ms;
            normalize(&mut dayno, &mut daytime);
        }
        Ok(self.new_date(dayno, daytime))
    }

    /// A date less a number of days, or the days between two dates, for
    /// `-`.
    pub(crate) fn date_sub(&mut self, id: ObjId, other: Value) -> VmResult<Value> {
        let (mut dayno, mut daytime) = self.as_date(Value::Obj(id)).unwrap_or_default();
        if let Some((other_day, other_time)) = self.as_date(other) {
            // The difference in days, as a Decimal.
            let day = |dayno: i32, daytime: i32| {
                let fraction = Big::div(
                    &Big::from_int(daytime.into(), 32),
                    &Big::from_int(MS_PER_DAY.into(), 32),
                )
                .unwrap_or_else(|| Big::zero(32));
                Big::add(&Big::from_int(dayno.into(), 32), &fraction)
            };
            let difference = Big::sub(&day(dayno, daytime), &day(other_day, other_time));
            return Ok(self.new_big(difference));
        }
        if let Value::Int(days) = other {
            dayno = dayno.wrapping_sub(days);
        } else {
            let (whole, ms) = self.day_count(other)?;
            let difference = whole - f64::from(dayno);
            if difference > f64::from(i32::MAX) || difference < f64::from(i32::MIN) {
                return Err(VmError::new(code::NUM_OVERFLOW));
            }
            dayno = dayno.wrapping_sub(whole as i32);
            daytime -= ms;
            normalize(&mut dayno, &mut daytime);
        }
        Ok(self.new_date(dayno, daytime))
    }

    /// The text of a date for display: the date and time in the local
    /// zone.
    pub(crate) fn date_text(&mut self, id: ObjId) -> String {
        let (mut dayno, mut daytime) = self.as_date(Value::Obj(id)).unwrap_or_default();
        let utc = (dayno, daytime);
        let (abbr, offset) = self.local_zone().utc_to_local(&mut dayno, &mut daytime);
        let mut calendar = Calendar::new(false);
        calendar.set_dayno(dayno);
        let locale = self.locale();
        let mut out = Vec::new();
        format_date(
            &mut out,
            b"%Y-%m-%d %H:%M:%S",
            &locale,
            &calendar,
            (dayno, daytime),
            utc,
            &abbr,
            offset,
        );
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Call a method of Date on a date, or with no object on the class
    /// itself.
    pub(crate) fn date_method(
        &mut self,
        id: Option<ObjId>,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let Some(&desc) = DATE_METHODS.get(index as usize).filter(|_| index != 0) else {
            return Ok(None);
        };
        let class_method = matches!(index, 1 | 2 | 13);
        if id.is_none() && !class_method {
            return Ok(None);
        }
        self.native_method(argc, desc, |vm, argc| match id {
            Some(id) if !class_method => vm.date_call(id, index, argc),
            _ => vm.date_class_call(index, argc),
        })
        .map(Some)
    }

    fn date_class_call(&mut self, index: u16, argc: u32) -> VmResult<Value> {
        if index == 13 {
            // setLocaleInfo: a list of every setting, or pairs of a
            // setting's number and its value.
            if argc == 1 {
                if let Some(len) = self.list_like_len(self.arg(0))? {
                    for item in 0..len {
                        let value = self.list_like_get(self.arg(0), item + 1)?;
                        self.set_locale(item as i32, value)?;
                    }
                    return Ok(Value::Nil);
                }
            }
            if argc % 2 != 0 {
                return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
            }
            for pair in (0..argc as usize).step_by(2) {
                let item = self.cast_to_int(self.arg(pair))?;
                self.set_locale(item, self.arg(pair + 1))?;
            }
            return Ok(Value::Nil);
        }

        // parseDate(str, format?, refDate?, refTZ?) and parseJulianDate.
        let julian = index == 2;
        let Some(text) = self.as_str(self.arg(0)).map(|text| text.to_vec()) else {
            return Err(VmError::new(code::BAD_VAL_BIF));
        };
        let custom = if argc >= 2 { self.arg(1) } else { Value::Nil };
        let reference = if argc >= 3 && self.arg(2) != Value::Nil {
            self.as_date(self.arg(2))
                .ok_or_else(|| VmError::new(code::BAD_TYPE_BIF))?
        } else {
            self.now()
        };
        let zone = self.zone_arg(3, argc)?;
        let Some(ParsedDate {
            dayno,
            daytime,
            parsed,
            formats,
        }) = self.parse_date_string(&text, custom, julian, reference, &zone)?
        else {
            return Ok(Value::Nil);
        };
        // The date, the zone if the text named one, the offset if it
        // gave one, the formats that matched, and the text of each part.
        let list = self.new_list(vec![Value::Nil; 5]);
        self.push(list);
        let date = self.new_date(dayno, daytime);
        let parsed_zone = match &parsed.zone {
            Some(found) if !Rc::ptr_eq(found, &zone) => {
                Value::Obj(self.alloc_obj(Object::TimeZone(found.clone())))
            }
            _ => Value::Nil,
        };
        let offset = if parsed.tzofs != UNSET {
            Value::Int(parsed.tzofs / 1000)
        } else {
            Value::Nil
        };
        let format_list = self.new_list(vec![Value::Nil; formats.len()]);
        let items = formats
            .iter()
            .map(|format| self.new_string(format.clone()))
            .collect();
        self.set_list(format_list, items);
        let part_list = self.new_list(vec![Value::Nil; parsed.captures.len()]);
        let items = parsed
            .captures
            .iter()
            .map(|capture| match capture.start {
                Some(start) => {
                    let end = (start + capture.len).min(text.len());
                    self.new_string(text[start.min(end)..end].to_vec())
                }
                None => Value::Nil,
            })
            .collect();
        self.set_list(part_list, items);
        self.set_list(
            list,
            vec![date, parsed_zone, offset, format_list, part_list],
        );
        self.pop();
        Ok(list)
    }

    fn int_list(&mut self, items: &[i32]) -> Value {
        self.new_list(items.iter().map(|&n| Value::Int(n)).collect::<Vec<_>>())
    }

    fn date_call(&mut self, id: ObjId, index: u16, argc: u32) -> VmResult<Value> {
        let Some((dayno, daytime)) = self.as_date(Value::Obj(id)) else {
            return Ok(Value::Nil);
        };
        // The date as a zone sees it.
        let local = |vm: &mut Self, arg: usize| -> VmResult<(i32, i32, Rc<Zone>)> {
            let zone = vm.zone_arg(arg, argc)?;
            let (mut day, mut time) = (dayno, daytime);
            zone.utc_to_local(&mut day, &mut time);
            Ok((day, time, zone))
        };
        Ok(match index {
            // formatDate(format, tz?) and formatJulianDate
            3 | 4 => {
                let Some(format) = self.as_str(self.arg(0)).map(|text| text.to_vec()) else {
                    return Err(VmError::new(code::BAD_TYPE_BIF));
                };
                let zone = self.zone_arg(1, argc)?;
                let (mut day, mut time) = (dayno, daytime);
                let (abbr, offset) = zone.utc_to_local(&mut day, &mut time);
                let mut calendar = Calendar::new(index == 4);
                calendar.set_dayno(day);
                let locale = self.locale();
                let mut out = Vec::new();
                format_date(
                    &mut out,
                    &format,
                    &locale,
                    &calendar,
                    (day, time),
                    (dayno, daytime),
                    &abbr,
                    offset,
                );
                self.try_new_string(out)?
            }
            // compareTo(date)
            5 => {
                let Some((other_day, other_time)) = self.as_date(self.arg(0)) else {
                    return Err(VmError::new(code::BAD_TYPE_BIF));
                };
                Value::Int(if dayno != other_day {
                    dayno.wrapping_sub(other_day)
                } else {
                    daytime.wrapping_sub(other_time)
                })
            }
            // getDate(tz?): year, month, day, and weekday from 1 for
            // Sunday.
            6 => {
                let date = CalDate::from_dayno(local(self, 0)?.0);
                self.int_list(&[date.y, date.m, date.d, date.weekday() + 1])
            }
            // getJulianDay()
            7 => {
                let day = Big::parse(
                    &format!("{:.1}", CalDate::from_dayno(dayno).julian_dayno()),
                    32,
                );
                let fraction = Big::div(
                    &Big::from_int(daytime.into(), 32),
                    &Big::from_int(MS_PER_DAY.into(), 32),
                )
                .unwrap_or_else(|| Big::zero(32));
                self.new_big(Big::add(&day, &fraction))
            }
            // getJulianDate(tz?)
            8 => {
                let date = CalDate::from_dayno(local(self, 0)?.0);
                let (y, m, d) = date.julian_date();
                self.int_list(&[y, m, d, date.weekday() + 1])
            }
            // getISOWeekDate(tz?)
            9 => {
                let date = CalDate::from_dayno(local(self, 0)?.0);
                let (week, year) = date.iso_weekno();
                self.int_list(&[year, week, date.iso_weekday()])
            }
            // getClockTime(tz?). The reference VM's arithmetic here
            // gives a zero, the hour, the minute, and the milliseconds.
            10 => {
                let time = local(self, 0)?.1;
                self.int_list(&[
                    time / MS_PER_DAY,
                    time / (60 * 60 * 1000) % 60,
                    time / (60 * 1000) % 60,
                    time % 1000,
                ])
            }
            // addInterval([years, months, days, hours, minutes, seconds])
            11 => {
                let interval = self.arg(0);
                let Some(len) = self.list_like_len(interval)? else {
                    return Err(VmError::new(code::BAD_TYPE_BIF));
                };
                let mut date = CalDate::from_dayno(dayno);
                let mut time = f64::from(daytime);
                for item in 1..=len {
                    let value = self.list_like_get(interval, item)?;
                    if value == Value::Nil {
                        continue;
                    }
                    match item {
                        1 => date.y = date.y.wrapping_add(self.cast_to_int(value)?),
                        2 => date.m = date.m.wrapping_add(self.cast_to_int(value)?),
                        3 => date.d = date.d.wrapping_add(self.cast_to_int(value)?),
                        4 => time += self.num_to_f64(value)? * 3_600_000.0,
                        5 => time += self.num_to_f64(value)? * 60_000.0,
                        6 => time += self.num_to_f64(value)? * 1000.0,
                        _ => {}
                    }
                }
                let carry = (time / DAY_MS).floor();
                time -= carry * DAY_MS;
                let day = carry + f64::from(date.d);
                if day > f64::from(i32::MAX) || day < f64::from(i32::MIN) {
                    return Err(VmError::new(code::NUM_OVERFLOW));
                }
                date.d = day as i32;
                self.new_date(date.dayno(), time as i32)
            }
            // findWeekday(weekday, which, tz?)
            12 => {
                let weekday = self.cast_to_int(self.arg(0))?;
                let which = self.cast_to_int(self.arg(1))?;
                if !(1..=7).contains(&weekday) || which == 0 {
                    return Err(VmError::new(code::BAD_VAL_BIF));
                }
                let (mut day, _, zone) = local(self, 2)?;
                let delta = (7 + (weekday - 1) - CalDate::from_dayno(day).weekday()) % 7;
                if which > 0 {
                    day = day.wrapping_add(delta + (which - 1).wrapping_mul(7));
                } else {
                    day = day.wrapping_sub((7 - delta) % 7 + (-which - 1).wrapping_mul(7));
                }
                let mut time = 0;
                zone.local_to_utc(&mut day, &mut time);
                self.new_date(day, time)
            }
            _ => Value::Nil,
        })
    }
}

/// Write a number: at least so many digits, padded with zeros, or with
/// `pad` if there is one, or not at all for the `#` flag.
fn write_number(
    out: &mut Vec<u8>,
    value: i32,
    digits: usize,
    pound: bool,
    pad: Option<u8>,
    roman: bool,
) {
    let negative = value < 0;
    if negative {
        out.push(b'-');
    }
    let magnitude = value.unsigned_abs();
    if roman && !negative && (1..=4999).contains(&magnitude) {
        const NUMERALS: [(&str, u32); 13] = [
            ("M", 1000),
            ("CM", 900),
            ("D", 500),
            ("CD", 400),
            ("C", 100),
            ("XC", 90),
            ("L", 50),
            ("X", 10),
            ("IX", 9),
            ("V", 5),
            ("IV", 4),
            ("I", 1),
            ("", 0),
        ];
        let mut rest = magnitude;
        for (numeral, worth) in NUMERALS {
            while worth != 0 && rest >= worth {
                out.extend_from_slice(numeral.as_bytes());
                rest -= worth;
            }
        }
        return;
    }
    let text = magnitude.to_string();
    if text.len() < digits && (pad.is_some() || !pound) {
        out.extend(std::iter::repeat_n(
            pad.unwrap_or(b'0'),
            digits - text.len(),
        ));
    }
    out.extend_from_slice(text.as_bytes());
}

/// Write a date according to a format, in which `%` and a letter stand
/// for a part of the date, as in C's `strftime`. `local` is the day and
/// time in the zone the date is written for, and `utc` the same moment in
/// UTC.
#[allow(clippy::too_many_arguments)]
fn format_date(
    out: &mut Vec<u8>,
    format: &[u8],
    locale: &Locale,
    date: &Calendar,
    local: (i32, i32),
    utc: (i32, i32),
    zone_abbr: &str,
    zone_offset: i32,
) {
    let (dayno, daytime) = local;
    let mut at = 0;
    while at < format.len() {
        let c = format[at];
        if c != b'%' || format.len() - at < 2 {
            out.push(c);
            at += 1;
            continue;
        }
        at += 1;
        // Flags, as long as a letter is left after them.
        let (mut pound, mut minus, mut roman, mut pad) = (false, false, false, None);
        loop {
            let remaining = format.len() - at;
            let mut found = false;
            let flag = |wanted: &[u8], at: &mut usize| {
                let hit = format.len() - *at >= 2 && wanted.contains(&format[*at]);
                if hit {
                    *at += 1;
                }
                hit
            };
            if flag(b"#", &mut at) {
                pound = true;
                found = true;
            }
            if format.len() - at >= 2 && matches!(format[at], b' ' | 0x15) {
                pad = Some(format[at]);
                at += 1;
                found = true;
            }
            if flag(b"-", &mut at) {
                minus = true;
                found = true;
            }
            if flag(b"&", &mut at) {
                roman = true;
                found = true;
            }
            if !found || remaining < 2 {
                break;
            }
        }
        let code = format[at];
        at += 1;
        let number = |out: &mut Vec<u8>, value: i32, digits: usize| {
            write_number(out, value, digits, pound, pad, roman);
        };
        let item = |out: &mut Vec<u8>, setting: usize, index: i32, sticky: bool| {
            out.extend_from_slice(locale.index_list(setting, index, sticky));
        };
        let sub_format = |out: &mut Vec<u8>, format: &[u8]| {
            format_date(
                out,
                format,
                locale,
                date,
                local,
                utc,
                zone_abbr,
                zone_offset,
            );
        };
        match code {
            b'a' => item(out, LC_WKDY, date.date.weekday(), false),
            b'A' => item(out, LC_WEEKDAY, date.date.weekday(), false),
            b'd' => number(out, date.d, 2),
            b'j' => number(
                out,
                dayno.wrapping_sub(CalDate::new(date.y, 1, 1).dayno()) + 1,
                3,
            ),
            b'J' => {
                // The Julian day number, which starts at noon.
                let (mut day, mut time) = (i64::from(utc.0), i64::from(utc.1));
                if time > 12 * 60 * 60 * 1000 {
                    day += 1;
                    time -= 12 * 60 * 60 * 1000;
                }
                let fraction = Big::div(
                    &Big::from_int(time, 32),
                    &Big::from_int(MS_PER_DAY.into(), 32),
                )
                .unwrap_or_else(|| Big::zero(32));
                let julian = Big::add(&Big::from_int(day + 1_721_119, 32), &fraction);
                let text = julian.format(-1, -1, 10, -1, decimal::FORMAT_POINT, None);
                out.extend_from_slice(text.as_bytes());
            }
            b'u' => number(out, date.date.iso_weekday(), 1),
            b'w' => number(out, date.date.weekday(), 1),
            b't' => {
                number(out, date.d, 1);
                item(out, LC_ORDSUF, ordinal_index(date.d), true);
            }
            b'U' => number(out, date.weekno(0), 2),
            b'W' => number(out, date.weekno(1), 2),
            b'V' => number(out, date.date.iso_weekno().0, 2),
            b'b' => item(out, LC_MON, date.m - 1, false),
            b'B' => item(out, LC_MONTH, date.m - 1, false),
            b'm' => number(out, date.m, 2),
            b'C' => number(out, date.y / 100, 2),
            b'g' | b'G' => number(
                out,
                date.date.iso_weekno().1,
                if code == b'g' { 2 } else { 4 },
            ),
            b'y' => number(out, date.y % 100, 2),
            b'Y' => number(out, date.y, 4),
            b'e' | b'E' => {
                // The year with its era, which goes before the year or
                // after it.
                let (era, year) = if date.y <= 0 {
                    (1, -date.y + 1)
                } else {
                    (0, date.y)
                };
                let era_first = (code == b'e' && minus)
                    || (code == b'E' && !minus && era == 0)
                    || (code == b'E' && minus && era == 1);
                if era_first {
                    item(out, LC_ERA, era, false);
                    out.push(b' ');
                    number(out, year, 1);
                } else {
                    number(out, year, 1);
                    out.push(b' ');
                    item(out, LC_ERA, era, false);
                }
            }
            b'H' => number(out, daytime / (60 * 60 * 1000), 2),
            b'I' => {
                let hour = match daytime / (60 * 60 * 1000) {
                    0 => 12,
                    hour if hour >= 13 => hour - 12,
                    hour => hour,
                };
                number(out, hour, 2);
            }
            b'M' => number(out, daytime / (60 * 1000) % 60, 2),
            b'P' => item(
                out,
                LC_AMPM,
                (daytime / (60 * 60 * 1000) >= 12).into(),
                false,
            ),
            b'p' => item(
                out,
                LC_AMPM,
                if daytime / (60 * 60 * 1000) >= 12 {
                    3
                } else {
                    2
                },
                false,
            ),
            b'S' => number(out, daytime / 1000 % 60, 2),
            b'N' => number(out, daytime % 1000, 3),
            b'z' => out.extend_from_slice(zone_abbr.as_bytes()),
            b'Z' => {
                out.push(if zone_offset < 0 { b'-' } else { b'+' });
                let offset = zone_offset.abs();
                number(out, offset / (60 * 60 * 1000), 2);
                write_number(out, offset / (60 * 1000) % 60, 2, false, pad, roman);
            }
            b's' => {
                let seconds = (f64::from(dayno) - f64::from(UNIX_EPOCH_DAYNO)) * 86400.0
                    + f64::from(daytime / 1000)
                    - f64::from(zone_offset / 1000);
                out.extend_from_slice(format!("{seconds:.0}").as_bytes());
            }
            b'r' => sub_format(out, locale.get(LC_FMT_12HOUR)),
            b'R' => sub_format(out, locale.get(LC_FMT_24HOUR)),
            b'T' => sub_format(out, locale.get(LC_FMT_24HOURSECS)),
            b'X' => sub_format(out, locale.get(LC_FMT_TIME)),
            b'c' => sub_format(out, locale.get(LC_FMT_TIMESTAMP)),
            b'D' => sub_format(out, locale.get(LC_FMT_SHORTDATE)),
            b'x' => sub_format(out, locale.get(LC_FMT_DATE)),
            b'F' => sub_format(out, b"%Y-%m-%d"),
            b'%' => out.push(b'%'),
            other => {
                out.push(b'%');
                out.push(other);
            }
        }
    }
}
