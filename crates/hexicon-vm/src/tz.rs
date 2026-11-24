//! Time zones: the zoneinfo database, and zones made up from an offset or
//! a POSIX-style description.
//!
//! The database is the IANA zoneinfo data as the reference VM's tools
//! compile it, converted to a layout of Hexicon's own and embedded whole.
//! For each zone it has the history of changes to the zone's offset from
//! UTC, and the rules that carry the pattern of daylight saving time on
//! past the last change recorded.

use std::collections::HashMap;
use std::rc::Rc;

use crate::date::CalDate;

// The time zones, from the IANA's zoneinfo data, in a layout of Hexicon's
// own. Numbers are little-endian, and a name is a byte giving its length
// and then the name.
//
//     "HXTZ", and the format version as a 32-bit integer: 1
//     the number of zones, of links, and of abbreviations, 32 bits each
//     for each zone: its name, the two letters of its country (or zeros),
//         and the offset in the file of its record, 32 bits
//     for each link, which is another name for a zone: the name, and the
//         zone's index, 32 bits
//     for each abbreviation: its name, a count as a byte, and for each
//         zone that uses it: the zone's index, 32 bits; the offset from
//         UTC in milliseconds, 32 bits, signed; and "S", "D", or "B" for
//         standard time, daylight time, or both
//     for each zone, its record: the record's length, 16 bits, and then
//         the number of transitions, 16 bits
//         the number of time types, 16 bits
//         the number of rules, a byte
//         the length of the abbreviations, a byte
//         for each transition: the day number and the time of day in
//             milliseconds, 32 bits each, signed, and the index of the
//             time type it changes to, a byte
//         for each rule: the offset of its abbreviation, a byte; the
//             month; how the day is chosen; the day; the weekday, a byte
//             each; the time of day in milliseconds, 24 bits, and a byte
//             saying which clock that is on; and the offset from UTC and
//             the daylight saving, in milliseconds, 32 bits each, signed
//         for each time type: the offset from UTC and the daylight
//             saving, 32 bits each, signed, and the offset of its
//             abbreviation, a byte
//         the abbreviations, each ended by a zero byte
//         a zero byte, or else the country as two letters, the
//             coordinates as 15 characters and a zero byte, and a comment
//             as a length byte, the text, and a zero byte
static DATABASE: &[u8] = include_bytes!("../data/timezones.bin");

const MS_PER_DAY: i32 = 24 * 60 * 60 * 1000;

/// A combination of offset and abbreviation that a zone uses.
#[derive(Debug, Clone)]
pub(crate) struct TimeType {
    /// Standard time's offset from UTC, in milliseconds.
    pub gmtofs: i32,
    /// What daylight saving time adds to that.
    pub save: i32,
    /// The abbreviation: `EST`.
    pub abbr: Rc<str>,
}

/// A moment at which a zone changed to another time type.
#[derive(Debug, Clone)]
pub(crate) struct Transition {
    pub dayno: i32,
    pub daytime: i32,
    pub time_type: usize,
}

/// A rule for a change that happens every year.
#[derive(Debug, Clone)]
pub(crate) struct Rule {
    pub abbr: Rc<str>,
    pub gmtofs: i32,
    pub save: i32,
    pub month: i32,
    /// How `day` and `weekday` pick the day: 0 for the day of the month,
    /// 1 for the last such weekday of the month, 2 for the first on or
    /// after the day, 3 for the last on or before it.
    pub when: u8,
    pub day: i32,
    /// 1 for Sunday through 7 for Saturday.
    pub weekday: i32,
    /// The time of day, in milliseconds.
    pub at: i32,
    /// What clock `at` is on: 0x80 for UTC, 0x40 for local standard time,
    /// 0 for local wall clock time.
    pub at_zone: u8,
}

/// What a zone's clocks were doing at some moment.
#[derive(Debug, Clone)]
pub(crate) struct Query {
    /// When this state of affairs began, or the minimum values if it
    /// always was so.
    pub start_dayno: i32,
    pub start_daytime: i32,
    pub gmtofs: i32,
    pub save: i32,
    pub abbr: Rc<str>,
}

/// A date and time carried so that the time is within one day.
pub(crate) fn normalize(dayno: &mut i32, daytime: &mut i32) {
    *dayno = dayno.wrapping_add(daytime.div_euclid(MS_PER_DAY));
    *daytime = daytime.rem_euclid(MS_PER_DAY);
}

/// A daylight saving rule as a POSIX-style zone description gives it.
#[derive(Debug, Clone, Default)]
struct DescRule {
    /// Day of the year from 1, not counting February 29.
    jday: i32,
    /// Day of the year from 1, counting February 29.
    yday: i32,
    month: i32,
    week: i32,
    /// 1 for Sunday through 7.
    day: i32,
    /// Seconds after midnight.
    time: i32,
}

impl DescRule {
    fn is_set(&self) -> bool {
        self.jday != 0 || self.yday != 0 || self.month != 0
    }
}

/// A zone described by its offsets and abbreviations, in seconds.
#[derive(Debug, Clone, Default)]
pub(crate) struct ZoneDesc {
    pub std_ofs: i32,
    pub dst_ofs: i32,
    pub std_abbr: String,
    pub dst_abbr: String,
    dst_start: DescRule,
    dst_end: DescRule,
}

impl ZoneDesc {
    /// The description of a zone from the offsets, in milliseconds, and
    /// the abbreviation recorded with it.
    pub(crate) fn recorded(gmtofs: i32, save: i32, abbr: &str) -> Self {
        Self {
            std_ofs: gmtofs / 1000,
            dst_ofs: gmtofs / 1000 + save / 1000,
            std_abbr: abbr.to_owned(),
            dst_abbr: abbr.to_owned(),
            ..Self::default()
        }
    }
}

/// A time zone.
#[derive(Debug)]
pub(crate) struct Zone {
    /// The name the zone is listed under, if it is listed.
    key: Option<String>,
    /// Other names the database has for the zone, the last listed first.
    links: Vec<String>,
    transitions: Vec<Transition>,
    rules: Vec<Rule>,
    time_types: Vec<TimeType>,
    /// The country the zone is in, as two letters, if the database says.
    pub country: String,
    /// Latitude and longitude of the zone's main city: `+404251-0740023`.
    pub coords: String,
    /// A comment on what part of the country the zone covers.
    pub desc: Option<String>,
    /// For a zone read from an image or a saved state and not yet looked
    /// up: the offsets and abbreviation recorded with its name.
    pub pending: Option<(i32, i32, String)>,
}

impl Zone {
    /// A zone from its record in the database.
    fn from_database(key: &str, links: Vec<String>, data: &[u8]) -> Option<Self> {
        let u16_at = |at: usize| {
            Some(usize::from(u16::from_le_bytes([
                *data.get(at)?,
                *data.get(at + 1)?,
            ])))
        };
        let i32_at = |at: usize| {
            let bytes = data.get(at..at + 4)?;
            Some(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        };
        let transition_count = u16_at(0)?;
        let type_count = u16_at(2)?;
        let rule_count = usize::from(*data.get(4)?);
        let abbr_len = usize::from(*data.get(5)?);
        let transitions_at = 6;
        let rules_at = transitions_at + transition_count * 9;
        let types_at = rules_at + rule_count * 17;
        let abbrs_at = types_at + type_count * 9;
        let abbrs = data.get(abbrs_at..abbrs_at + abbr_len)?;
        // An abbreviation is the text from an offset up to a null.
        let abbr = |offset: u8| -> Rc<str> {
            let rest = abbrs.get(usize::from(offset)..).unwrap_or_default();
            let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
            String::from_utf8_lossy(&rest[..end]).into()
        };

        let mut transitions = Vec::with_capacity(transition_count);
        for index in 0..transition_count {
            let at = transitions_at + index * 9;
            transitions.push(Transition {
                dayno: i32_at(at)?,
                daytime: i32_at(at + 4)?,
                time_type: usize::from(*data.get(at + 8)?).min(type_count.saturating_sub(1)),
            });
        }
        let mut rules = Vec::with_capacity(rule_count);
        for index in 0..rule_count {
            let at = rules_at + index * 17;
            let time = i32_at(at + 5)?;
            rules.push(Rule {
                abbr: abbr(*data.get(at)?),
                month: i32::from(*data.get(at + 1)?),
                when: *data.get(at + 2)?,
                day: i32::from(*data.get(at + 3)?),
                weekday: i32::from(*data.get(at + 4)?),
                at: time & 0x00ff_ffff,
                at_zone: (time >> 24) as u8,
                gmtofs: i32_at(at + 9)?,
                save: i32_at(at + 13)?,
            });
        }
        let mut time_types = Vec::with_capacity(type_count);
        for index in 0..type_count {
            let at = types_at + index * 9;
            time_types.push(TimeType {
                gmtofs: i32_at(at)?,
                save: i32_at(at + 4)?,
                abbr: abbr(*data.get(at + 8)?),
            });
        }
        if time_types.is_empty() {
            return None;
        }

        // The country, the coordinates, and a comment, if there are any.
        let mut zone = Self {
            key: Some(key.to_owned()),
            links,
            transitions,
            rules,
            time_types,
            country: String::new(),
            coords: String::new(),
            desc: None,
            pending: None,
        };
        let at = abbrs_at + abbr_len;
        if data.get(at).is_some_and(|&b| b != 0) {
            let text = |range: std::ops::Range<usize>| {
                let bytes = data.get(range).unwrap_or_default();
                let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
                String::from_utf8_lossy(&bytes[..end]).into_owned()
            };
            zone.country = text(at..at + 2);
            zone.coords = text(at + 2..at + 17);
            let len = usize::from(*data.get(at + 18)?);
            zone.desc = Some(text(at + 19..at + 19 + len));
        }
        Some(zone)
    }

    /// A zone from a description, listed under a name.
    fn from_desc(key: &str, desc: &ZoneDesc) -> Self {
        let std_abbr: Rc<str> = desc.std_abbr.as_str().into();
        let dst_abbr: Rc<str> = desc.dst_abbr.as_str().into();
        let mut rules = Vec::new();
        let time_types = if desc.dst_start.is_set() && desc.dst_end.is_set() {
            let save = (desc.dst_ofs - desc.std_ofs) * 1000;
            let rule = |from: &DescRule, abbr: &Rc<str>, save: i32| {
                let mut rule = Rule {
                    abbr: abbr.clone(),
                    gmtofs: desc.std_ofs * 1000,
                    save,
                    month: 0,
                    when: 0,
                    day: 0,
                    weekday: 0,
                    at: from.time * 1000,
                    at_zone: 0,
                };
                if from.jday != 0 {
                    // The day of a year without a leap day.
                    let mut date = CalDate::new(2011, 1, from.jday);
                    date.normalize();
                    rule.month = date.m;
                    rule.day = date.d;
                } else if from.yday != 0 {
                    rule.month = 1;
                    rule.day = from.yday;
                } else {
                    rule.month = from.month;
                    rule.weekday = from.day;
                    if from.week == 5 {
                        rule.when = 1;
                    } else {
                        rule.when = 2;
                        rule.day = (from.week - 1) * 7 + 1;
                    }
                }
                rule
            };
            rules.push(rule(&desc.dst_start, &dst_abbr, save));
            rules.push(rule(&desc.dst_end, &std_abbr, 0));
            vec![
                TimeType {
                    gmtofs: desc.std_ofs * 1000,
                    save: 0,
                    abbr: std_abbr,
                },
                TimeType {
                    gmtofs: desc.dst_ofs * 1000,
                    save,
                    abbr: dst_abbr,
                },
            ]
        } else {
            vec![TimeType {
                gmtofs: desc.std_ofs * 1000,
                save: 0,
                abbr: std_abbr,
            }]
        };
        Self {
            key: Some(key.to_owned()),
            links: Vec::new(),
            transitions: Vec::new(),
            rules,
            time_types,
            country: String::new(),
            coords: String::new(),
            desc: None,
            pending: None,
        }
    }

    /// A zone as an image or a saved state records it: a name, and the
    /// offsets in milliseconds and abbreviation it had. It stands in for
    /// the zone until the name is looked up.
    pub(crate) fn recorded(name: &str, gmtofs: i32, save: i32, abbr: &str) -> Self {
        let desc = ZoneDesc::recorded(gmtofs, save, abbr);
        let mut zone = Self::from_desc(name, &desc);
        zone.pending = Some((gmtofs, save, abbr.to_owned()));
        zone
    }

    /// The zone's name.
    pub(crate) fn name(&self) -> &str {
        match &self.key {
            Some(key) => key,
            None => &self.time_types[0].abbr,
        }
    }

    /// Every name the zone goes by, the main one first.
    pub(crate) fn names(&self) -> Vec<String> {
        let mut names = vec![self.name().to_owned()];
        names.extend(self.links.iter().cloned());
        names
    }

    pub(crate) fn transitions(&self) -> &[Transition] {
        &self.transitions
    }

    pub(crate) fn time_type(&self, index: usize) -> &TimeType {
        &self.time_types[index.min(self.time_types.len() - 1)]
    }

    pub(crate) fn rules(&self) -> &[Rule] {
        &self.rules
    }

    fn query_of_type(&self, index: usize, dayno: i32, daytime: i32) -> Query {
        let time_type = self.time_type(index);
        Query {
            start_dayno: dayno,
            start_daytime: daytime,
            gmtofs: time_type.gmtofs,
            save: time_type.save,
            abbr: time_type.abbr.clone(),
        }
    }

    fn query_of_transition(&self, index: usize) -> Query {
        let transition = &self.transitions[index];
        self.query_of_type(transition.time_type, transition.dayno, transition.daytime)
    }

    /// How a transition compares with a time: negative if it is earlier.
    /// With `local`, the time is on the zone's own clock as the transition
    /// leaves it.
    fn compare_transition(&self, index: usize, dayno: i32, daytime: i32, local: bool) -> i64 {
        let transition = &self.transitions[index];
        let (mut day, mut time) = (transition.dayno, transition.daytime);
        if local {
            let time_type = self.time_type(transition.time_type);
            time += time_type.gmtofs + time_type.save;
            normalize(&mut day, &mut time);
        }
        if day != dayno {
            i64::from(day) - i64::from(dayno)
        } else {
            i64::from(time) - i64::from(daytime)
        }
    }

    /// What the zone's clocks were doing at a time, which is in UTC, or
    /// with `local` on the zone's own clock.
    pub(crate) fn query(&self, dayno: i32, daytime: i32, local: bool) -> Query {
        let count = self.transitions.len();
        // After the last recorded change, the rules take over.
        if count == 0 || self.compare_transition(count - 1, dayno, daytime, local) <= 0 {
            if !self.rules.is_empty() {
                let year = CalDate::from_dayno(dayno).y;
                // The latest rule to have taken effect, looking back as
                // far as two years.
                for yy in (year - 2..=year + 1).rev() {
                    let mut latest: Option<(usize, i32, i32)> = None;
                    for (index, rule) in self.rules.iter().enumerate() {
                        let previous =
                            &self.rules[index.checked_sub(1).unwrap_or(self.rules.len() - 1)];
                        let (mut day, mut time) = rule.resolve(yy, previous);
                        if local {
                            time += rule.gmtofs + rule.save;
                            normalize(&mut day, &mut time);
                        }
                        let passed = dayno > day || (dayno == day && daytime > time);
                        if passed && latest.is_none_or(|(_, latest_day, _)| day > latest_day) {
                            latest = Some((index, day, time));
                        }
                    }
                    if let Some((index, day, time)) = latest {
                        let rule = &self.rules[index];
                        return Query {
                            start_dayno: day,
                            start_daytime: time,
                            gmtofs: rule.gmtofs,
                            save: rule.save,
                            abbr: rule.abbr.clone(),
                        };
                    }
                }
            }
            return if count != 0 {
                self.query_of_transition(count - 1)
            } else {
                self.query_of_type(0, i32::MIN, i32::MIN)
            };
        }
        // Before the first, the zone was on its first time type.
        if self.compare_transition(0, dayno, daytime, local) >= 0 {
            return self.query_of_type(0, i32::MIN, i32::MIN);
        }
        // Search for the latest transition before the time. This is the
        // reference VM's search, step for step.
        let (mut lo, mut hi) = (0usize, count - 1);
        while lo < hi {
            let cur = (hi + lo) / 2;
            let direction = self.compare_transition(cur, dayno, daytime, local);
            if direction < 0 {
                lo = cur + 1;
            } else if direction > 0 {
                hi = cur.saturating_sub(1);
            } else {
                // The moment of a change belongs to what came before it.
                return self.query_of_transition(cur.saturating_sub(1));
            }
        }
        let cur = if self.compare_transition(lo, dayno, daytime, local) >= 0
            && lo > 0
            && self.compare_transition(lo - 1, dayno, daytime, local) < 0
        {
            lo - 1
        } else {
            lo
        };
        self.query_of_transition(cur)
    }

    /// Change a UTC time to the zone's time: the abbreviation in force
    /// and the offset in milliseconds.
    pub(crate) fn utc_to_local(&self, dayno: &mut i32, daytime: &mut i32) -> (Rc<str>, i32) {
        let query = self.query(*dayno, *daytime, false);
        let offset = query.gmtofs + query.save;
        *daytime += offset;
        normalize(dayno, daytime);
        (query.abbr, offset)
    }

    /// Change a time on the zone's clock to UTC.
    pub(crate) fn local_to_utc(&self, dayno: &mut i32, daytime: &mut i32) {
        let query = self.query(*dayno, *daytime, true);
        *daytime -= query.gmtofs + query.save;
        normalize(dayno, daytime);
    }
}

impl Rule {
    /// When the rule takes effect in a year, in UTC. The rule in effect
    /// before it says what the local clock reads until then.
    fn resolve(&self, year: i32, previous: &Rule) -> (i32, i32) {
        let mut date = CalDate::new(year, self.month, 0);
        // The weekday counted from 1 for Sunday, as the rules have it.
        let weekday_of = |date: &CalDate| date.weekday_by_dayno() + 1;
        match self.when {
            0 => date.d = self.day,
            1 => {
                // Day zero of the next month is the last of this one.
                date.m += 1;
                let delta = (weekday_of(&date) - self.weekday).rem_euclid(7);
                date.d -= delta;
            }
            2 => {
                date.d = self.day;
                let delta = (self.weekday - weekday_of(&date)).rem_euclid(7);
                date.d += delta;
            }
            3 => {
                date.d = self.day;
                let delta = (self.weekday - weekday_of(&date)).rem_euclid(7);
                date.d += delta - 7;
            }
            _ => {}
        }
        let mut dayno = date.dayno();
        let mut daytime = self.at;
        match self.at_zone {
            0x80 => {}
            0x40 => daytime -= previous.gmtofs,
            0 => daytime -= previous.gmtofs + previous.save,
            _ => {}
        }
        normalize(&mut dayno, &mut daytime);
        (dayno, daytime)
    }
}

/// A zone the database lists.
#[derive(Debug)]
struct Listed {
    name: String,
    /// Where the zone's record is in the database.
    position: usize,
    country: [u8; 2],
    /// Other names for the zone, the last listed first.
    links: Vec<String>,
    zone: Option<Rc<Zone>>,
}

/// One zone that uses an abbreviation.
#[derive(Debug, Clone, Copy)]
struct AbbrUse {
    zone: Option<usize>,
    gmtofs: i32,
    /// `S` for standard time, `D` for daylight time, `B` for both.
    kind: u8,
}

/// The zones the VM knows, each loaded the first time it is asked for.
#[derive(Debug, Default)]
pub(crate) struct ZoneCache {
    loaded: bool,
    listed: Vec<Listed>,
    /// Zone and link names, in lower case, to the zone's index.
    names: HashMap<String, usize>,
    abbrs: HashMap<String, Vec<AbbrUse>>,
    /// Zones made from descriptions, by name in lower case.
    made: HashMap<String, Rc<Zone>>,
    local: Option<Rc<Zone>>,
}

/// A name in one of the database's tables: a length byte and the text.
/// Returns the name and the position after it.
fn table_name(data: &[u8], at: usize) -> Option<(String, usize)> {
    let len = usize::from(*data.get(at)?);
    let name = String::from_utf8_lossy(data.get(at + 1..at + 1 + len)?).into_owned();
    Some((name, at + 1 + len))
}

impl ZoneCache {
    /// Read the database's tables of names.
    fn load_index(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let _ = self.read_index();
    }

    fn read_index(&mut self) -> Option<()> {
        let data = DATABASE;
        let u32_at = |at: usize| {
            let bytes = data.get(at..at + 4)?;
            Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize)
        };
        if data.get(..4)? != b"HXTZ" || u32_at(4)? != 1 {
            return None;
        }
        let (zone_count, link_count, abbr_count) = (u32_at(8)?, u32_at(12)?, u32_at(16)?);

        let mut at = 20;
        for _ in 0..zone_count {
            let (name, after) = table_name(data, at)?;
            let country = data.get(after..after + 2)?;
            self.names.insert(name.to_lowercase(), self.listed.len());
            self.listed.push(Listed {
                name,
                position: u32_at(after + 2)?,
                country: [country[0], country[1]],
                links: Vec::new(),
                zone: None,
            });
            at = after + 6;
        }
        for _ in 0..link_count {
            let (name, after) = table_name(data, at)?;
            let target = u32_at(after)?;
            if let Some(listed) = self.listed.get_mut(target) {
                self.names.entry(name.to_lowercase()).or_insert(target);
                listed.links.insert(0, name);
            }
            at = after + 4;
        }
        for _ in 0..abbr_count {
            let (name, after) = table_name(data, at)?;
            let count = usize::from(*data.get(after)?);
            let mut uses = Vec::with_capacity(count);
            let mut entry = after + 1;
            for _ in 0..count {
                let zone = u32_at(entry)?;
                uses.push(AbbrUse {
                    zone: (zone < self.listed.len()).then_some(zone),
                    gmtofs: u32_at(entry + 4)? as u32 as i32,
                    kind: *data.get(entry + 8)?,
                });
                entry += 9;
            }
            self.abbrs.insert(name.to_lowercase(), uses);
            at = entry;
        }
        Some(())
    }

    /// The listed zone at an index, loading it if need be.
    fn listed_zone(&mut self, index: usize) -> Option<Rc<Zone>> {
        let listed = self.listed.get_mut(index)?;
        if listed.zone.is_none() {
            listed.zone = Self::load_zone(listed).map(Rc::new);
        }
        listed.zone.clone()
    }

    fn load_zone(listed: &Listed) -> Option<Zone> {
        let size = DATABASE.get(listed.position..listed.position + 2)?;
        let size = usize::from(u16::from_le_bytes([size[0], size[1]]));
        let record = DATABASE.get(listed.position + 2..listed.position + 2 + size)?;
        Zone::from_database(&listed.name, listed.links.clone(), record)
    }

    /// The zone the database lists under a name, in any case.
    pub(crate) fn database_zone(&mut self, name: &str) -> Option<Rc<Zone>> {
        self.load_index();
        let lower = name.to_lowercase();
        // A zone that was missing from the database when it was first
        // named is remembered alongside the ones that are there.
        if let Some(zone) = self.made.get(&format!("db:{lower}")) {
            return Some(zone.clone());
        }
        let index = *self.names.get(&lower)?;
        self.listed_zone(index)
    }

    /// A zone made from a description, kept under its name.
    fn made_zone(&mut self, name: &str, desc: &ZoneDesc) -> Rc<Zone> {
        self.made
            .entry(name.to_lowercase())
            .or_insert_with(|| Rc::new(Zone::from_desc(name, desc)))
            .clone()
    }

    /// Stand in for a zone that a saved state names and the database
    /// lacks.
    pub(crate) fn missing_zone(&mut self, name: &str, desc: &ZoneDesc) -> Rc<Zone> {
        self.made
            .entry(format!("db:{}", name.to_lowercase()))
            .or_insert_with(|| Rc::new(Zone::from_desc(name, desc)))
            .clone()
    }

    /// The zone at a fixed offset from UTC, in seconds.
    pub(crate) fn offset_zone(&mut self, seconds: i32) -> Rc<Zone> {
        let name = offset_name("UTC", seconds, true, true);
        let desc = ZoneDesc {
            std_ofs: seconds,
            std_abbr: name.clone(),
            ..ZoneDesc::default()
        };
        self.made_zone(&name, &desc)
    }

    /// The zone a name refers to, in any of the forms a program can use:
    /// `:local`, `UTC`, an offset such as `+0500` or `UTC-8`, a
    /// POSIX-style description such as `EST5EDT`, or a database name.
    pub(crate) fn parse_zone(
        &mut self,
        name: &str,
        local_key: &mut dyn FnMut() -> LocalZone,
    ) -> Option<Rc<Zone>> {
        if name.is_empty() || name == ":local" {
            return Some(self.local_zone(local_key));
        }
        if ["z", "utc", "gmt"]
            .iter()
            .any(|utc| name.eq_ignore_ascii_case(utc))
        {
            return Some(self.offset_zone(0));
        }
        let bytes = name.as_bytes();
        let offset = |text: &[u8]| {
            let mut rest = text;
            parse_hhmmss(&mut rest, true).filter(|_| rest.is_empty())
        };
        if let Some(seconds) = offset(bytes) {
            return Some(self.offset_zone(seconds));
        }
        if bytes.len() >= 3 && bytes[..3].eq_ignore_ascii_case(b"utc") {
            if let Some(seconds) = offset(&bytes[3..]) {
                return Some(self.offset_zone(seconds));
            }
        }
        if let Some(desc) = parse_posix(bytes) {
            return Some(self.made_zone(name, &desc));
        }
        self.database_zone(name)
    }

    /// The zone the host's clock is in. This is a zone object of its
    /// own, even when the database has the same zone, as it is in the
    /// reference VM, where a program can tell the two apart.
    pub(crate) fn local_zone(&mut self, local_key: &mut dyn FnMut() -> LocalZone) -> Rc<Zone> {
        if let Some(local) = &self.local {
            return local.clone();
        }
        self.load_index();
        let host = local_key();
        let listed = host
            .name
            .as_ref()
            .and_then(|name| self.names.get(&name.to_lowercase()).copied())
            .and_then(|index| self.listed.get(index))
            .and_then(Self::load_zone);
        let zone = match listed {
            Some(zone) => zone,
            None => {
                // All that is known is the offset.
                let name = offset_name("UTC", host.utc_offset_seconds, true, true);
                let desc = ZoneDesc {
                    std_ofs: host.utc_offset_seconds,
                    std_abbr: name.clone(),
                    ..ZoneDesc::default()
                };
                Zone::from_desc(&name, &desc)
            }
        };
        let zone = Rc::new(zone);
        self.local = Some(zone.clone());
        zone
    }

    /// Whether a zone is the local zone's own object.
    pub(crate) fn is_local(&self, zone: &Rc<Zone>) -> bool {
        self.local
            .as_ref()
            .is_some_and(|local| Rc::ptr_eq(local, zone))
    }

    /// The zone an abbreviation such as `EST` most likely means here, and
    /// the offset in milliseconds that the abbreviation itself implies,
    /// unless it is used for standard and daylight time alike.
    pub(crate) fn zone_by_abbr(
        &mut self,
        abbr: &str,
        now: (i32, i32),
        local_key: &mut dyn FnMut() -> LocalZone,
    ) -> Option<(Rc<Zone>, Option<i32>)> {
        self.load_index();
        let uses = self.abbrs.get(&abbr.to_lowercase())?.clone();
        let first = *uses.first()?;
        let local = self.local_zone(local_key);
        // Prefer a zone in the local zone's country, then one within
        // three hours of local time, then the first listed.
        let country = local.country.as_bytes();
        let in_country = |candidate: &AbbrUse| {
            country.len() >= 2
                && candidate
                    .zone
                    .and_then(|index| self.listed.get(index))
                    .is_some_and(|listed| listed.country == country[..2])
        };
        let local_offset = local.query(now.0, now.1, false).gmtofs;
        let near = |candidate: &AbbrUse| {
            (i64::from(local_offset) - i64::from(candidate.gmtofs)).abs() < 3 * 60 * 60 * 1000
        };
        let chosen = uses
            .iter()
            .copied()
            .find(in_country)
            .or_else(|| uses.iter().copied().find(near))
            .unwrap_or(first);
        let zone = self.listed_zone(chosen.zone?)?;
        Some((zone, (chosen.kind != b'B').then_some(chosen.gmtofs)))
    }
}

/// What the host knows about its own time zone.
#[derive(Debug, Clone, Default)]
pub(crate) struct LocalZone {
    /// The zone's name in the zoneinfo database, if the host knows it.
    pub name: Option<String>,
    /// The current offset from UTC, for when there is no name.
    pub utc_offset_seconds: i32,
}

/// A name for an offset: the prefix and then `+5`, `-3:30`, and so on.
fn offset_name(prefix: &str, seconds: i32, hide_zero: bool, plus_sign: bool) -> String {
    let (sign, seconds) = if seconds < 0 {
        ("-", -i64::from(seconds))
    } else if plus_sign {
        ("+", i64::from(seconds))
    } else {
        ("", i64::from(seconds))
    };
    let (hh, mm, ss) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if ss != 0 {
        format!("{prefix}{sign}{hh}:{mm:02}:{ss:02}")
    } else if mm != 0 {
        format!("{prefix}{sign}{hh}:{mm:02}")
    } else if hh != 0 || !hide_zero {
        format!("{prefix}{sign}{hh}")
    } else {
        prefix.to_owned()
    }
}

/// Digits at the start of text, as a number, taking them off the text.
fn take_digits(text: &mut &[u8]) -> (i32, usize) {
    let mut value: i32 = 0;
    let mut count = 0;
    while let [digit @ b'0'..=b'9', rest @ ..] = *text {
        value = value.wrapping_mul(10).wrapping_add(i32::from(digit - b'0'));
        count += 1;
        *text = rest;
    }
    (value, count)
}

/// Whether text starts with a colon and two digits.
fn colon_digits(text: &[u8]) -> bool {
    matches!(text, [b':', b'0'..=b'9', b'0'..=b'9', ..])
}

/// An offset as `+hh`, `+hhmm`, `+hh:mm`, or `+hh:mm:ss`, in seconds,
/// taking it off the text.
fn parse_hhmmss(text: &mut &[u8], sign_required: bool) -> Option<i32> {
    let mut rest = *text;
    let sign = match rest.first() {
        Some(b'+') => {
            rest = &rest[1..];
            1
        }
        Some(b'-') => {
            rest = &rest[1..];
            -1
        }
        _ if sign_required => return None,
        _ => 1,
    };
    let (mut hh, digits) = take_digits(&mut rest);
    if !(1..=4).contains(&digits) {
        return None;
    }
    let (mut mm, mut ss) = (0, 0);
    if digits >= 3 {
        mm = hh % 100;
        hh /= 100;
    } else if colon_digits(rest) {
        rest = &rest[1..];
        mm = take_digits(&mut rest).0;
        if colon_digits(rest) {
            rest = &rest[1..];
            ss = take_digits(&mut rest).0;
        }
    }
    *text = rest;
    Some(sign * (hh * 3600 + mm * 60 + ss))
}

/// An offset in a POSIX-style description: `5`, `-1`, `6:30`.
fn posix_offset(text: &mut &[u8]) -> Option<i32> {
    let mut rest = *text;
    let mut sign = 1;
    if let [b'+', b'0'..=b'9', ..] = rest {
        rest = &rest[1..];
    } else if let [b'-', b'0'..=b'9', ..] = rest {
        sign = -1;
        rest = &rest[1..];
    }
    if !rest.first().is_some_and(u8::is_ascii_digit) {
        return None;
    }
    let hh = take_digits(&mut rest).0;
    let (mut mm, mut ss) = (0, 0);
    if colon_digits(rest) {
        rest = &rest[1..];
        mm = take_digits(&mut rest).0;
        if colon_digits(rest) {
            rest = &rest[1..];
            ss = take_digits(&mut rest).0;
        }
    }
    *text = rest;
    Some(sign * (hh * 3600 + mm * 60 + ss))
}

/// A daylight saving rule in a POSIX-style description: `J60`, `59`, or
/// `M3.2.0`, each optionally followed by `/` and a time.
fn posix_rule(text: &mut &[u8]) -> Option<DescRule> {
    let mut rest = *text;
    let mut rule = DescRule::default();
    let digit_next = |rest: &[u8]| rest.get(1).is_some_and(u8::is_ascii_digit);
    match rest.first() {
        Some(b'J' | b'j') if digit_next(rest) => {
            rest = &rest[1..];
            rule.jday = take_digits(&mut rest).0;
        }
        Some(b'0'..=b'9') => {
            rule.yday = take_digits(&mut rest).0 + 1;
        }
        Some(b'M') if digit_next(rest) => {
            rest = &rest[1..];
            rule.month = take_digits(&mut rest).0;
            if !(1..=12).contains(&rule.month) || rest.first() != Some(&b'.') || !digit_next(rest) {
                return None;
            }
            rest = &rest[1..];
            rule.week = take_digits(&mut rest).0;
            if !(1..=5).contains(&rule.week) || rest.first() != Some(&b'.') || !digit_next(rest) {
                return None;
            }
            rest = &rest[1..];
            rule.day = take_digits(&mut rest).0 + 1;
            if !(1..=7).contains(&rule.day) {
                return None;
            }
        }
        _ => return None,
    }
    rule.time = 2 * 60 * 60;
    if rest.first() == Some(&b'/') && digit_next(rest) {
        rest = &rest[1..];
        if let Some(time) = posix_offset(&mut rest) {
            rule.time = time;
        }
    }
    *text = rest;
    Some(rule)
}

/// A zone described POSIX style: a name, an offset, and optionally a
/// daylight name, its offset, and the rules for when it starts and ends.
/// The reference VM takes the offsets as east of UTC, the opposite of
/// POSIX, so `EST5` is five hours ahead.
fn parse_posix(text: &[u8]) -> Option<ZoneDesc> {
    let mut rest = text.trim_ascii_start();
    let name = |rest: &mut &[u8]| {
        let len = rest.iter().take_while(|b| b.is_ascii_alphabetic()).count();
        let name = String::from_utf8_lossy(&rest[..len]).into_owned();
        *rest = &rest[len..];
        name
    };
    let mut desc = ZoneDesc {
        std_abbr: name(&mut rest),
        ..ZoneDesc::default()
    };
    if desc.std_abbr.is_empty() {
        return None;
    }
    desc.std_ofs = posix_offset(&mut rest)?;
    desc.dst_abbr = name(&mut rest);
    desc.dst_ofs = if desc.dst_abbr.is_empty() {
        desc.std_ofs
    } else {
        posix_offset(&mut rest).unwrap_or(desc.std_ofs + 3600)
    };
    if rest.first() == Some(&b',') {
        rest = &rest[1..];
        desc.dst_start = posix_rule(&mut rest)?;
        if rest.first() != Some(&b',') {
            return None;
        }
        rest = &rest[1..];
        desc.dst_end = posix_rule(&mut rest)?;
    }
    // Nothing can follow, not even a space.
    rest.is_empty().then_some(desc)
}
