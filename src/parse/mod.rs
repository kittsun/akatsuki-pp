mod attributes;
mod control_point;
mod error;
mod hitobject;
mod hitsound;
mod pos2;
mod sort;

pub use attributes::BeatmapAttributes;
pub use control_point::{DifficultyPoint, TimingPoint};
pub use error::{ParseError, ParseResult};
pub use hitobject::{HitObject, HitObjectKind};
pub use hitsound::HitSound;
pub use pos2::Pos2;
use sort::legacy_sort;

use std::cmp::Ordering;

#[cfg(not(any(feature = "async_std", feature = "async_tokio")))]
use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
};

#[cfg(feature = "async_tokio")]
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncRead, BufReader},
};

#[cfg(not(feature = "async_std"))]
use std::path::Path;

#[cfg(feature = "async_std")]
use async_std::{
    fs::File,
    io::{prelude::BufReadExt, BufReader as AsyncBufReader, Read as AsyncRead},
    path::Path,
};

#[cfg(feature = "sliders")]
pub use osu_fruits::*;

fn sort_unstable<T: PartialOrd>(slice: &mut [T]) {
    slice.sort_unstable_by(|p1, p2| p1.partial_cmp(p2).unwrap_or(Ordering::Equal));
}

trait OptionExt<T> {
    fn next_field(self, field: &'static str) -> Result<T, ParseError>;
}

impl<T> OptionExt<T> for Option<T> {
    fn next_field(self, field: &'static str) -> Result<T, ParseError> {
        self.ok_or(ParseError::MissingField(field))
    }
}

trait FloatExt: Sized {
    fn validate(self) -> Result<Self, ParseError>;
}

impl FloatExt for f64 {
    fn validate(self) -> Result<Self, ParseError> {
        self.is_finite()
            .then(|| self)
            .ok_or(ParseError::InvalidDecimalNumber)
    }
}

macro_rules! line_prepare {
    ($buf:ident) => {{
        let mut line = $buf.trim_end();

        if skip_line(line) {
            $buf.clear();
            continue;
        }

        if let Some(idx) = line.find("//") {
            line = &line[..idx];
        }

        line
    }};
}

macro_rules! section {
    ($map:ident, $func:ident, $reader:ident, $buf:ident, $section:ident) => {{
        #[cfg(not(any(feature = "async_std", feature = "async_tokio")))]
        if $map.$func(&mut $reader, &mut $buf, &mut $section)? {
            break;
        }

        #[cfg(any(feature = "async_std", feature = "async_tokio"))]
        if $map.$func(&mut $reader, &mut $buf, &mut $section).await? {
            break;
        }
    }};
}

macro_rules! read_line {
    ($reader:ident, $buf:expr) => {{
        #[cfg(any(feature = "async_std", feature = "async_tokio"))]
        {
            $reader.read_line($buf).await
        }

        #[cfg(not(any(feature = "async_std", feature = "async_tokio")))]
        {
            $reader.read_line($buf)
        }
    }};
}

macro_rules! parse_general_body {
    ($self:ident, $reader:ident, $buf:ident, $section:ident) => {{
        let mut mode = None;
        let mut empty = true;

        #[cfg(feature = "osu")]
        let mut stack_leniency = None;

        while read_line!($reader, $buf)? != 0 {
            let line = line_prepare!($buf);

            if line.starts_with('[') && line.ends_with(']') {
                *$section = Section::from_str(&line[1..line.len() - 1]);
                empty = false;
                $buf.clear();
                break;
            }

            let (key, value) = split_colon(&line).ok_or(ParseError::BadLine)?;

            if key == "Mode" {
                mode = match value {
                    "0" => Some(GameMode::STD),
                    "1" => Some(GameMode::TKO),
                    "2" => Some(GameMode::CTB),
                    "3" => Some(GameMode::MNA),
                    _ => return Err(ParseError::InvalidMode),
                };
            }

            #[cfg(feature = "osu")]
            if key == "StackLeniency" {
                stack_leniency = Some(value.parse()?);
            }

            $buf.clear();
        }

        $self.mode = mode.unwrap_or(GameMode::STD);

        #[cfg(not(feature = "osu"))]
        if $self.mode == GameMode::STD {
            return Err(ParseError::UnincludedMode(GameMode::STD));
        }

        #[cfg(not(feature = "taiko"))]
        if $self.mode == GameMode::TKO {
            return Err(ParseError::UnincludedMode(GameMode::TKO));
        }

        #[cfg(not(feature = "fruits"))]
        if $self.mode == GameMode::CTB {
            return Err(ParseError::UnincludedMode(GameMode::CTB));
        }

        #[cfg(not(feature = "mania"))]
        if $self.mode == GameMode::MNA {
            return Err(ParseError::UnincludedMode(GameMode::MNA));
        }

        #[cfg(feature = "osu")]
        {
            $self.stack_leniency = stack_leniency.unwrap_or(0.7);
        }

        Ok(empty)
    }};
}

macro_rules! parse_general {
    () => {
        fn parse_general<R: Read>(
            &mut self,
            reader: &mut BufReader<R>,
            buf: &mut String,
            section: &mut Section,
        ) -> ParseResult<bool> {
            parse_general_body!(self, reader, buf, section)
        }
    };

    (async $reader:ident<$inner:ident>) => {
        async fn parse_general<R: $inner + Unpin>(
            &mut self,
            reader: &mut $reader<R>,
            buf: &mut String,
            section: &mut Section,
        ) -> ParseResult<bool> {
            parse_general_body!(self, reader, buf, section)
        }
    };
}

macro_rules! parse_difficulty_body {
    ($self:ident, $reader:ident, $buf:ident, $section:ident) => {{
        let mut ar = None;
        let mut od = None;
        let mut cs = None;
        let mut hp = None;
        let mut sv = None;
        let mut tick_rate = None;

        let mut empty = true;

        while read_line!($reader, $buf)? != 0 {
            let line = line_prepare!($buf);

            if line.starts_with('[') && line.ends_with(']') {
                *$section = Section::from_str(&line[1..line.len() - 1]);
                empty = false;
                $buf.clear();
                break;
            }

            let (key, value) = split_colon(&line).ok_or(ParseError::BadLine)?;

            match key {
                "ApproachRate" => ar = Some(value.parse()?),
                "OverallDifficulty" => od = Some(value.parse()?),
                "CircleSize" => cs = Some(value.parse()?),
                "HPDrainRate" => hp = Some(value.parse()?),
                "SliderTickRate" => tick_rate = Some(value.parse()?),
                "SliderMultiplier" => sv = Some(value.parse()?),
                _ => {}
            }

            $buf.clear();
        }

        $self.od = od.next_field("od")?;
        $self.cs = cs.next_field("cs")?;
        $self.hp = hp.next_field("hp")?;
        $self.ar = ar.unwrap_or($self.od);
        $self.slider_mult = sv.next_field("sv")?;
        $self.tick_rate = tick_rate.next_field("tick rate")?;

        Ok(empty)
    }};
}

macro_rules! parse_difficulty {
    () => {
        fn parse_difficulty<R: Read>(
            &mut self,
            reader: &mut BufReader<R>,
            buf: &mut String,
            section: &mut Section,
        ) -> ParseResult<bool> {
            parse_difficulty_body!(self, reader, buf, section)
        }
    };

    (async $reader:ident<$inner:ident>) => {
        async fn parse_difficulty<R: $inner + Unpin>(
            &mut self,
            reader: &mut $reader<R>,
            buf: &mut String,
            section: &mut Section,
        ) -> ParseResult<bool> {
            parse_difficulty_body!(self, reader, buf, section)
        }
    };
}

macro_rules! parse_timingpoints_body {
    (short => $self:ident, $reader:ident, $buf:ident, $section:ident) => {{
        let mut empty = true;

        // Only parse the first timing point to calculate the bpm
        if read_line!($reader, $buf)? != 0 {
            let line = {
                let mut line = $buf.trim_end();

                if skip_line(line) {
                    $buf.clear();
                    return Ok(empty);
                }

                if let Some(idx) = line.find("//") {
                    line = &line[..idx];
                }

                line
            };

            if line.starts_with('[') && line.ends_with(']') {
                *$section = Section::from_str(&line[1..line.len() - 1]);
                empty = false;
                $buf.clear();
                return Ok(empty);
            }

            let beat_len = line
                .split(',')
                .nth(1)
                .next_field("beat_len")?
                .trim()
                .parse()?;

            $self.bpm = bpm(beat_len);

            $buf.clear();
        }

        while read_line!($reader, $buf)? != 0 {
            let line = line_prepare!($buf);

            if line.starts_with('[') && line.ends_with(']') {
                *$section = Section::from_str(&line[1..line.len() - 1]);
                empty = false;
                $buf.clear();
                break;
            }

            $buf.clear();
        }

        Ok(empty)
    }};

    ($self:ident, $reader:ident, $buf:ident, $section:ident) => {{
        let mut unsorted_timings = false;
        let mut unsorted_difficulties = false;

        let mut prev_diff = 0.0;
        let mut prev_time = 0.0;

        let mut empty = true;

        while read_line!($reader, $buf)? != 0 {
            let line = line_prepare!($buf);

            if line.starts_with('[') && line.ends_with(']') {
                *$section = Section::from_str(&line[1..line.len() - 1]);
                empty = false;
                $buf.clear();
                break;
            }

            let mut split = line.split(',');

            let time = split
                .next()
                .next_field("timing point time")?
                .trim()
                .parse::<f64>()?
                .validate()?;

            let beat_len: f64 = split.next().next_field("beat len")?.trim().parse()?;

            if beat_len < 0.0 {
                let point = DifficultyPoint {
                    time,
                    speed_multiplier: (-100.0 / beat_len).max(0.1).min(10.0),
                };

                $self.difficulty_points.push(point);

                if time < prev_diff {
                    unsorted_difficulties = true;
                } else {
                    prev_diff = time;
                }
            } else {
                $self.timing_points.push(TimingPoint { time, beat_len });

                if time < prev_time {
                    unsorted_timings = true;
                } else {
                    prev_time = time;
                }
            }

            $buf.clear();
        }

        if unsorted_timings {
            sort_unstable(&mut $self.timing_points);
        }

        if unsorted_difficulties {
            sort_unstable(&mut $self.difficulty_points);
        }

        Ok(empty)
    }};
}

macro_rules! parse_timingpoints {
    () => {
        fn parse_timingpoints<R: Read>(
            &mut self,
            reader: &mut BufReader<R>,
            buf: &mut String,
            section: &mut Section,
        ) -> ParseResult<bool> {
            #[cfg(not(feature = "sliders"))]
            {
                parse_timingpoints_body!(short => self, reader, buf, section)
            }

            #[cfg(feature = "sliders")]
            parse_timingpoints_body!(self, reader, buf, section)
        }
    };

    (async $reader:ident<$inner:ident>) => {
        async fn parse_timingpoints<R: $inner + Unpin>(
            &mut self,
            reader: &mut $reader<R>,
            buf: &mut String,
            section: &mut Section,
        ) -> ParseResult<bool> {
            #[cfg(not(feature = "sliders"))]
            {
                parse_timingpoints_body!(short => self, reader, buf, section)
            }

            #[cfg(feature = "sliders")]
            parse_timingpoints_body!(self, reader, buf, section)
        }
    };
}

macro_rules! parse_hitobjects_body {
    ($self:ident, $reader:ident, $buf:ident, $section:ident) => {{
        let mut unsorted = false;
        let mut prev_time = 0.0;
        let mut empty = true;

        // `point_split` will be of type `Vec<&str>
        // with each element having its lifetime bound to `buf`.
        // To circumvent this, `point_split_raw` will contain
        // the actual `&str` elements transmuted into `usize`.
        #[cfg(feature = "sliders")]
        let mut point_split_raw: Vec<usize> = Vec::new();

        // Buffer to re-use for all sliders
        #[cfg(feature = "sliders")]
        let mut vertices = Vec::new();

        while read_line!($reader, $buf)? != 0 {
            let line = line_prepare!($buf);

            if line.starts_with('[') && line.ends_with(']') {
                *$section = Section::from_str(&line[1..line.len() - 1]);
                empty = false;
                $buf.clear();
                break;
            }

            let mut split = line.split(',');

            let pos = Pos2 {
                x: split.next().next_field("x pos")?.parse()?,
                y: split.next().next_field("y pos")?.parse()?,
            };

            let time = split
                .next()
                .next_field("hitobject time")?
                .trim()
                .parse::<f64>()?
                .validate()?;

            if !$self.hit_objects.is_empty() && time < prev_time {
                unsorted = true;
            }

            let kind: u8 = split.next().next_field("hitobject kind")?.parse()?;
            let sound = split.next().map(str::parse).transpose()?.unwrap_or(0);

            let kind = if kind & Self::CIRCLE_FLAG > 0 {
                $self.n_circles += 1;

                HitObjectKind::Circle
            } else if kind & Self::SLIDER_FLAG > 0 {
                $self.n_sliders += 1;

                #[cfg(feature = "sliders")]
                {
                    let mut control_points = Vec::new();

                    let control_point_iter = split.next().next_field("control points")?.split('|');
                    let mut repeats: usize = split.next().next_field("repeats")?.parse()?;

                    if repeats > 9000 {
                        return Err(ParseError::TooManyRepeats);
                    }

                    // * osu-stable treated the first span of the slider
                    // * as a repeat, but no repeats are happening
                    repeats = repeats.saturating_sub(1);

                    let mut start_idx = 0;
                    let mut end_idx = 0;
                    let mut first = true;

                    // SAFETY: `Vec<usize>` and `Vec<&str>` have the same size and layout.
                    let point_split: &mut Vec<&str> =
                        unsafe { std::mem::transmute(&mut point_split_raw) };

                    point_split.clear();
                    point_split.extend(control_point_iter);

                    #[allow(clippy::blocks_in_if_conditions)]
                    while {
                        end_idx += 1;

                        end_idx < point_split.len()
                    } {
                        // * Keep incrementing end_idx while it's not the start of a new segment
                        // * (indicated by having a type descriptor of length 1).
                        if point_split[end_idx].len() > 1 {
                            continue;
                        }

                        // * Multi-segmented sliders DON'T contain the end point as part of the
                        // * current segment as it's assumed to be the start of the next segment.
                        // * The start of the next segment is the index after the type descriptor.
                        let end_point = point_split.get(end_idx + 1).copied();

                        convert_points(
                            &point_split[start_idx..end_idx],
                            end_point,
                            first,
                            pos,
                            &mut control_points,
                            &mut vertices,
                        )?;

                        start_idx = end_idx;
                        first = false;
                    }

                    if end_idx > start_idx {
                        convert_points(
                            &point_split[start_idx..end_idx],
                            None,
                            first,
                            pos,
                            &mut control_points,
                            &mut vertices,
                        )?;
                    }

                    if control_points.is_empty() {
                        HitObjectKind::Circle
                    } else {
                        let pixel_len = split
                            .next()
                            .next_field("pixel len")?
                            .parse::<f64>()?
                            .max(0.0)
                            .min(MAX_COORDINATE_VALUE);

                        HitObjectKind::Slider {
                            repeats,
                            pixel_len,
                            control_points,
                        }
                    }
                }

                #[cfg(not(feature = "sliders"))]
                {
                    let span_count = split.nth(1).next_field("repeats")?.parse()?;
                    let pixel_len = split.next().next_field("pixel len")?.parse()?;

                    HitObjectKind::Slider {
                        span_count,
                        pixel_len,
                    }
                }
            } else if kind & Self::SPINNER_FLAG > 0 {
                $self.n_spinners += 1;
                let end_time = split.next().next_field("spinner endtime")?.parse()?;

                HitObjectKind::Spinner { end_time }
            } else if kind & Self::HOLD_FLAG > 0 {
                $self.n_sliders += 1;
                let mut end = time;

                if let Some(next) = split.next() {
                    end = end.max(next.split(':').next().next_field("hold endtime")?.parse()?);
                }

                HitObjectKind::Hold { end_time: end }
            } else {
                return Err(ParseError::UnknownHitObjectKind);
            };

            $self.hit_objects.push(HitObject {
                pos,
                start_time: time,
                kind,
                sound,
            });

            prev_time = time;
            $buf.clear();
        }

        // BUG: If [General] section comes after [HitObjects] then the mode
        // won't be set yet so mania objects won't be sorted properly
        if $self.mode == GameMode::MNA {
            // First a _stable_ sort by time
            $self
                .hit_objects
                .sort_by(|p1, p2| p1.partial_cmp(p2).unwrap_or(Ordering::Equal));

            // Then the legacy sort for correct position order
            legacy_sort(&mut $self.hit_objects);
        } else if unsorted {
            sort_unstable(&mut $self.hit_objects);
        }

        Ok(empty)
    }};
}

macro_rules! parse_hitobjects {
    () => {
        fn parse_hitobjects<R: Read>(
            &mut self,
            reader: &mut BufReader<R>,
            buf: &mut String,
            section: &mut Section,
        ) -> ParseResult<bool> {
            parse_hitobjects_body!(self, reader, buf, section)
        }
    };

    (async $reader:ident<$inner:ident>) => {
        async fn parse_hitobjects<R: $inner + Unpin>(
            &mut self,
            reader: &mut $reader<R>,
            buf: &mut String,
            section: &mut Section,
        ) -> ParseResult<bool> {
            parse_hitobjects_body!(self, reader, buf, section)
        }
    };
}

macro_rules! parse_body {
    ($reader:ident<$inner:ident>: $input:ident) => {{
        let mut reader = $reader::new($input);
        let mut buf = String::new();

        while read_line!(reader, &mut buf)? != 0 {
            // Check for character U+FEFF specifically thanks to map id 797130
            if !buf
                .trim_matches(|c: char| c.is_whitespace() || c == '﻿')
                .is_empty()
            {
                break;
            }

            buf.clear();
        }

        let version = match buf.find(OSU_FILE_HEADER) {
            Some(idx) => buf[idx + OSU_FILE_HEADER.len()..].trim_end().parse()?,
            None => return Err(ParseError::IncorrectFileHeader),
        };

        buf.clear();

        let mut map = Beatmap {
            version,
            hit_objects: Vec::with_capacity(256),
            ..Default::default()
        };

        let mut section = Section::None;

        loop {
            match section {
                Section::General => section!(map, parse_general, reader, buf, section),
                Section::Difficulty => section!(map, parse_difficulty, reader, buf, section),
                Section::TimingPoints => section!(map, parse_timingpoints, reader, buf, section),
                Section::HitObjects => section!(map, parse_hitobjects, reader, buf, section),
                Section::None => {
                    if read_line!(reader, &mut buf)? == 0 {
                        break;
                    }

                    let line = line_prepare!(buf);

                    if line.starts_with('[') && line.ends_with(']') {
                        section = Section::from_str(&line[1..line.len() - 1]);
                    }

                    buf.clear();
                }
            }
        }

        Ok(map)
    }};
}

macro_rules! parse {
    () => {
        /// Parse a beatmap from a `.osu` file.
        ///
        /// As argument you can give anything that implements [`std::io::Read`].
        /// You'll likely want to pass (a reference of) a [`File`](std::fs::File)
        /// or the file's content as a slice of bytes (`&[u8]`).
        pub fn parse<R: Read>(input: R) -> ParseResult<Self> {
            parse_body!(BufReader<Read>: input)
        }
    };

    (async $reader:ident<$inner:ident>) => {
        /// Parse a beatmap from a `.osu` file.
        ///
        /// As argument you can give anything that implements `tokio::io::AsyncRead`
        /// or `async_std::io::Read`, depending which feature you chose.
        /// You'll likely want to pass a `File`
        /// or the file's content as a slice of bytes (`&[u8]`).
        pub async fn parse<R: $inner + Unpin>(input: R) -> ParseResult<Self> {
            parse_body!($reader<$inner>: input)
        }
    };
}

macro_rules! from_path {
    () => {
        /// Pass the path to a `.osu` file.
        ///
        /// Useful when you don't want to create the [`File`](std::fs::File) manually.
        /// If you have the file lying around already though (and plan on re-using it),
        /// passing `&file` to [`parse`](Beatmap::parse) should be preferred.
        pub fn from_path<P: AsRef<Path>>(path: P) -> ParseResult<Self> {
            Self::parse(File::open(path)?)
        }
    };

    (async $path:ident) => {
        /// Pass the path to a `.osu` file.
        ///
        /// Useful when you don't want to create the file manually.
        pub async fn from_path<P: AsRef<$path>>(path: P) -> ParseResult<Self> {
            Self::parse(File::open(path).await?).await
        }
    };
}

/// The mode of a beatmap.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq)]
#[allow(clippy::upper_case_acronyms)]
pub enum GameMode {
    /// osu!standard
    STD = 0,
    /// osu!taiko
    TKO = 1,
    /// osu!ctb
    CTB = 2,
    /// osu!mania
    MNA = 3,
}

impl Default for GameMode {
    #[inline]
    fn default() -> Self {
        Self::STD
    }
}

/// The main beatmap struct containing all data relevant
/// for difficulty and pp calculation
#[derive(Clone, Default, Debug)]
pub struct Beatmap {
    /// The game mode.
    pub mode: GameMode,
    /// The version of the .osu file.
    pub version: u8,

    /// The amount of circles.
    pub n_circles: u32,
    /// The amount of sliders.
    pub n_sliders: u32,
    /// The amount of spinners.
    pub n_spinners: u32,

    /// The approach rate.
    pub ar: f32,
    /// The overall difficulty.
    pub od: f32,
    /// The circle size.
    pub cs: f32,
    /// The health drain rate.
    pub hp: f32,
    /// Base slider velocity in pixels per beat
    pub slider_mult: f64,
    /// Amount of slider ticks per beat.
    pub tick_rate: f64,
    /// All hitobjects of the beatmap.
    pub hit_objects: Vec<HitObject>,

    #[cfg(not(feature = "sliders"))]
    /// Beats per minute
    pub bpm: f64,

    #[cfg(feature = "sliders")]
    /// Timing points that indicate a new timing section.
    pub timing_points: Vec<TimingPoint>,

    #[cfg(feature = "sliders")]
    /// Timing point for the current timing section.
    pub difficulty_points: Vec<DifficultyPoint>,

    #[cfg(feature = "osu")]
    /// The stack leniency that is used to calculate
    /// the stack offset for stacked positions.
    pub stack_leniency: f32,
}

pub(crate) const OSU_FILE_HEADER: &str = "osu file format v";

impl Beatmap {
    const CIRCLE_FLAG: u8 = 1 << 0;
    const SLIDER_FLAG: u8 = 1 << 1;
    // const NEW_COMBO_FLAG: u8 = 1 << 2;
    const SPINNER_FLAG: u8 = 1 << 3;
    // const COMBO_OFFSET_FLAG: u8 = (1 << 4) | (1 << 5) | (1 << 6);
    const HOLD_FLAG: u8 = 1 << 7;

    /// Extract a beatmap's attributes into their own type.
    #[inline]
    pub fn attributes(&self) -> BeatmapAttributes {
        BeatmapAttributes::new(self.ar, self.od, self.cs, self.hp)
    }

    /// The beats per minute of the map.
    #[cfg(feature = "sliders")]
    #[inline]
    pub fn bpm(&self) -> f64 {
        match self.timing_points.first() {
            Some(point) => bpm(point.beat_len),
            None => 0.0,
        }
    }

    /// The beats per minute of the map.
    #[cfg(not(feature = "sliders"))]
    #[inline]
    pub fn bpm(&self) -> f64 {
        self.bpm
    }
}

#[cfg(feature = "sliders")]
mod osu_fruits {
    use crate::ParseError;

    use super::Pos2;

    pub(super) const MAX_COORDINATE_VALUE: f64 = 131_072.0;

    pub(super) fn convert_points(
        points: &[&str],
        end_point: Option<&str>,
        first: bool,
        offset: Pos2,
        curve_points: &mut Vec<PathControlPoint>,
        vertices: &mut Vec<PathControlPoint>,
    ) -> Result<(), ParseError> {
        let mut path_kind = PathType::from_str(points[0]);

        let read_offset = first as usize;
        let readable_points = points.len() - 1;
        let end_point_len = end_point.is_some() as usize;

        vertices.clear();
        vertices.reserve(read_offset + readable_points + end_point_len);

        // * Fill any non-read points.
        vertices.extend((0..read_offset).map(|_| PathControlPoint::default()));

        // * Parse into control points.
        for &point in points.iter().skip(1) {
            vertices.push(read_point(point, offset)?);
        }

        // * If an endpoint is given, add it to the end.
        if let Some(end_point) = end_point {
            vertices.push(read_point(end_point, offset)?);
        }

        // * Edge-case rules (to match stable).
        if path_kind == PathType::PerfectCurve {
            if let [a, b, c] = &vertices[..] {
                if is_linear(a.pos, b.pos, c.pos) {
                    // * osu-stable special-cased colinear perfect curves to a linear path
                    path_kind = PathType::Linear;
                }
            } else {
                path_kind = PathType::Bezier;
            }
        }

        // * The first control point must have a definite type.
        vertices[0].kind = Some(path_kind);

        // * A path can have multiple implicit segments of the same type if
        // * there are two sequential control points with the same position.
        // * To handle such cases, this code may return multiple path segments
        // * with the final control point in each segment having a non-null type.
        // * For the point string X|1:1|2:2|2:2|3:3, this code returns the segments:
        // * X: { (1,1), (2, 2) }
        // * X: { (3, 3) }
        // * Note: (2, 2) is not returned in the second segments, as it is implicit in the path.
        let mut start_idx = 0;
        let mut end_idx = 0;

        #[allow(clippy::blocks_in_if_conditions)]
        while {
            end_idx += 1;

            end_idx < vertices.len() - end_point_len
        } {
            // * Keep incrementing while an implicit segment doesn't need to be started
            if vertices[end_idx].pos != vertices[end_idx - 1].pos {
                continue;
            }

            // * The last control point of each segment is not
            // * allowed to start a new implicit segment.
            if end_idx == vertices.len() - end_point_len - 1 {
                continue;
            }

            // * Force a type on the last point, and return
            // * the current control point set as a segment.
            vertices[end_idx - 1].kind = Some(path_kind);
            curve_points.extend(&vertices[start_idx..end_idx]);

            // * Skip the current control point - as it's the same as the one that's just been returned.
            start_idx = end_idx + 1;
        }

        if end_idx > start_idx {
            curve_points.extend(&vertices[start_idx..end_idx]);
        }

        Ok(())
    }

    pub(super) fn read_point(value: &str, start_pos: Pos2) -> Result<PathControlPoint, ParseError> {
        let mut v = value.split(':').map(str::parse);

        match (v.next(), v.next()) {
            (Some(Ok(x)), Some(Ok(y))) => Ok(PathControlPoint::from(Pos2 { x, y } - start_pos)),
            _ => Err(ParseError::InvalidCurvePoints),
        }
    }

    fn is_linear(p0: Pos2, p1: Pos2, p2: Pos2) -> bool {
        ((p1.x - p0.x) * (p2.y - p0.y) - (p1.y - p0.y) * (p2.x - p0.x)).abs() <= f32::EPSILON
    }

    /// Control point for slider curve calculation
    #[derive(Copy, Clone, Debug, Default, PartialEq)]
    pub struct PathControlPoint {
        /// Control point position.
        pub pos: Pos2,
        /// Path type of the control point.
        /// Only present for the first element of each segment.
        pub kind: Option<PathType>,
    }

    impl From<Pos2> for PathControlPoint {
        #[inline]
        fn from(pos: Pos2) -> Self {
            Self { pos, kind: None }
        }
    }

    /// The type of curve of a slider.
    #[allow(missing_docs)]
    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub enum PathType {
        Catmull = 0,
        Bezier = 1,
        Linear = 2,
        PerfectCurve = 3,
    }

    impl PathType {
        #[inline]
        fn from_str(s: &str) -> Self {
            match s {
                "L" => Self::Linear,
                "B" => Self::Bezier,
                "P" => Self::PerfectCurve,
                _ => Self::Catmull,
            }
        }
    }
}

#[cfg(not(any(feature = "async_std", feature = "async_tokio")))]
impl Beatmap {
    parse!();
    parse_general!();
    parse_difficulty!();
    parse_timingpoints!();
    parse_hitobjects!();

    from_path!();
}

#[cfg(feature = "async_tokio")]
impl Beatmap {
    parse!(async BufReader<AsyncRead>);
    parse_general!(async BufReader<AsyncRead>);
    parse_difficulty!(async BufReader<AsyncRead>);
    parse_timingpoints!(async BufReader<AsyncRead>);
    parse_hitobjects!(async BufReader<AsyncRead>);

    from_path!(async Path);
}

#[cfg(feature = "async_std")]
impl Beatmap {
    parse!(async AsyncBufReader<AsyncRead>);
    parse_general!(async AsyncBufReader<AsyncRead>);
    parse_difficulty!(async AsyncBufReader<AsyncRead>);
    parse_timingpoints!(async AsyncBufReader<AsyncRead>);
    parse_hitobjects!(async AsyncBufReader<AsyncRead>);

    from_path!(async Path);
}

fn bpm(beat_len: f64) -> f64 {
    beat_len.recip() * 1000.0 * 60.0
}

fn skip_line(line: &str) -> bool {
    line.is_empty() || line.starts_with("//") || line.starts_with(' ') || line.starts_with('_')
}

#[inline]
fn split_colon(line: &str) -> Option<(&str, &str)> {
    let mut split = line.split(':');

    Some((split.next()?, split.next()?.trim()))
}

#[derive(Copy, Clone, Debug)]
enum Section {
    None,
    General,
    Difficulty,
    TimingPoints,
    HitObjects,
}

impl Section {
    #[inline]
    fn from_str(s: &str) -> Self {
        match s {
            "General" => Self::General,
            "Difficulty" => Self::Difficulty,
            "TimingPoints" => Self::TimingPoints,
            "HitObjects" => Self::HitObjects,
            _ => Self::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(any(feature = "async_std", feature = "async_tokio")))]
    #[test]
    fn parsing_sync() {
        for map_id in map_ids() {
            println!("map_id: {}", map_id);

            let map = match Beatmap::from_path(format!("./maps/{}.osu", map_id)) {
                Ok(map) => map,
                Err(why) => panic!("Error while parsing map: {}", why),
            };

            print_info(map);
            println!("---");
        }
    }

    #[cfg(feature = "async_tokio")]
    #[test]
    fn parsing_async_tokio() {
        use tokio::runtime::Builder;

        Builder::new_current_thread()
            .build()
            .expect("could not start runtime")
            .block_on(async {
                for map_id in map_ids() {
                    println!("map_id: {}", map_id);

                    let map = match Beatmap::from_path(format!("./maps/{}.osu", map_id)).await {
                        Ok(map) => map,
                        Err(why) => panic!("Error while parsing map: {}", why),
                    };

                    print_info(map);
                    println!("---");
                }
            });
    }

    #[cfg(feature = "async_std")]
    #[test]
    fn parsing_async_std() {
        async_std::task::block_on(async {
            for map_id in map_ids() {
                println!("map_id: {}", map_id);

                let map = match Beatmap::from_path(format!("./maps/{}.osu", map_id)).await {
                    Ok(map) => map,
                    Err(why) => panic!("Error while parsing map: {}", why),
                };

                print_info(map);
                println!("---");
            }
        });
    }

    fn map_ids() -> Vec<i32> {
        let mut map_ids = Vec::new();

        if cfg!(feature = "osu") {
            map_ids.push(2785319);
        }

        if cfg!(feature = "mania") {
            map_ids.push(1974394);
        }

        if cfg!(feature = "fruits") {
            map_ids.push(2118524);
        }

        if cfg!(feature = "taiko") {
            map_ids.push(1028484);
        }

        map_ids
    }

    fn print_info(map: Beatmap) {
        println!("Mode: {}", map.mode as u8);
        println!("n_circles: {}", map.n_circles);
        println!("n_sliders: {}", map.n_sliders);
        println!("n_spinners: {}", map.n_spinners);
        println!("ar: {}", map.ar);
        println!("od: {}", map.od);
        println!("cs: {}", map.cs);
        println!("hp: {}", map.hp);
        println!("slider_mult: {}", map.slider_mult);
        println!("tick_rate: {}", map.tick_rate);
        println!("hit_objects: {}", map.hit_objects.len());

        #[cfg(feature = "sliders")]
        {
            #[cfg(feature = "osu")]
            println!("stack_leniency: {}", map.stack_leniency);

            println!("timing_points: {}", map.timing_points.len());
            println!("difficulty_points: {}", map.difficulty_points.len());
        }
    }
}
