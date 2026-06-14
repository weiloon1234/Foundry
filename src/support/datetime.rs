use std::fmt;
use std::str::FromStr;

use chrono::{
    DateTime as ChronoDateTime, Duration as ChronoDuration, FixedOffset, NaiveDate as ChronoDate,
    NaiveDateTime as ChronoNaiveDateTime, NaiveTime as ChronoTime, TimeZone as ChronoTimeZone,
    Timelike, Utc as ChronoUtc,
};
use chrono_tz::Tz;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::foundation::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DateTime(ChronoDateTime<ChronoUtc>);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDateTime(ChronoNaiveDateTime);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date(ChronoDate);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time(ChronoTime);

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Timezone {
    #[default]
    Utc,
    Iana(Tz),
    FixedOffset(FixedOffset),
}

#[derive(Clone, Debug)]
pub struct Clock {
    timezone: Timezone,
}

impl DateTime {
    pub fn now() -> Self {
        Self(ChronoUtc::now())
    }

    pub fn parse(value: impl AsRef<str>) -> Result<Self> {
        ChronoDateTime::parse_from_rfc3339(value.as_ref())
            .map(|value| Self(value.with_timezone(&ChronoUtc)))
            .map_err(|error| Error::message(format!("invalid datetime: {error}")))
    }

    pub fn parse_in_timezone(value: impl AsRef<str>, timezone: &Timezone) -> Result<Self> {
        let value = value.as_ref();
        if let Ok(value) = Self::parse(value) {
            return Ok(value);
        }
        LocalDateTime::parse(value)?.in_timezone(timezone)
    }

    pub fn format(&self) -> String {
        self.0.to_rfc3339()
    }

    pub fn format_in(&self, timezone: &Timezone) -> String {
        timezone.to_fixed_offset_datetime(self.0).to_rfc3339()
    }

    pub fn date_in(&self, timezone: &Timezone) -> Date {
        Date::from_chrono(timezone.to_fixed_offset_datetime(self.0).date_naive())
    }

    pub fn local_datetime_in(&self, timezone: &Timezone) -> LocalDateTime {
        LocalDateTime::from_chrono(timezone.to_fixed_offset_datetime(self.0).naive_local())
    }

    pub fn add_seconds(self, seconds: i64) -> Self {
        Self(self.0 + ChronoDuration::seconds(seconds))
    }

    pub fn sub_seconds(self, seconds: i64) -> Self {
        Self(self.0 - ChronoDuration::seconds(seconds))
    }

    pub fn add_days(self, days: i64) -> Self {
        Self(self.0 + ChronoDuration::days(days))
    }

    pub fn sub_days(self, days: i64) -> Self {
        Self(self.0 - ChronoDuration::days(days))
    }

    pub fn timestamp_millis(&self) -> i64 {
        self.0.timestamp_millis()
    }

    pub fn timestamp_micros(&self) -> i64 {
        self.0.timestamp_micros()
    }

    pub(crate) fn from_chrono(value: ChronoDateTime<ChronoUtc>) -> Self {
        Self(value)
    }

    pub(crate) fn as_chrono(&self) -> ChronoDateTime<ChronoUtc> {
        self.0
    }
}

impl fmt::Display for DateTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.format())
    }
}

impl FromStr for DateTime {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl Serialize for DateTime {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.format())
    }
}

impl<'de> Deserialize<'de> for DateTime {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl LocalDateTime {
    pub fn parse(value: impl AsRef<str>) -> Result<Self> {
        let value = value.as_ref();
        parse_local_datetime(value)
            .map(Self)
            .map_err(|error| Error::message(format!("invalid local datetime: {error}")))
    }

    pub fn format(&self) -> String {
        format_naive_datetime(self.0)
    }

    pub fn in_timezone(&self, timezone: &Timezone) -> Result<DateTime> {
        timezone
            .local_datetime_to_utc(self.0)
            .map(DateTime::from_chrono)
    }

    pub fn date(&self) -> Date {
        Date::from_chrono(self.0.date())
    }

    pub fn time(&self) -> Time {
        Time::from_chrono(self.0.time())
    }

    pub fn add_seconds(self, seconds: i64) -> Self {
        Self(self.0 + ChronoDuration::seconds(seconds))
    }

    pub fn sub_seconds(self, seconds: i64) -> Self {
        Self(self.0 - ChronoDuration::seconds(seconds))
    }

    pub fn add_days(self, days: i64) -> Self {
        Self(self.0 + ChronoDuration::days(days))
    }

    pub fn sub_days(self, days: i64) -> Self {
        Self(self.0 - ChronoDuration::days(days))
    }

    pub(crate) fn from_chrono(value: ChronoNaiveDateTime) -> Self {
        Self(value)
    }

    pub(crate) fn as_chrono(&self) -> ChronoNaiveDateTime {
        self.0
    }
}

impl fmt::Display for LocalDateTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.format())
    }
}

impl FromStr for LocalDateTime {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl Serialize for LocalDateTime {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.format())
    }
}

impl<'de> Deserialize<'de> for LocalDateTime {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl Date {
    pub fn parse(value: impl AsRef<str>) -> Result<Self> {
        ChronoDate::parse_from_str(value.as_ref(), "%Y-%m-%d")
            .map(Self)
            .map_err(|error| Error::message(format!("invalid date: {error}")))
    }

    pub fn format(&self) -> String {
        self.0.format("%Y-%m-%d").to_string()
    }

    pub(crate) fn from_chrono(value: ChronoDate) -> Self {
        Self(value)
    }

    pub(crate) fn as_chrono(&self) -> ChronoDate {
        self.0
    }
}

impl fmt::Display for Date {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.format())
    }
}

impl FromStr for Date {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl Serialize for Date {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.format())
    }
}

impl<'de> Deserialize<'de> for Date {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl Time {
    pub fn parse(value: impl AsRef<str>) -> Result<Self> {
        parse_time(value.as_ref())
            .map(Self)
            .map_err(|error| Error::message(format!("invalid time: {error}")))
    }

    pub fn format(&self) -> String {
        format_time(self.0)
    }

    pub(crate) fn from_chrono(value: ChronoTime) -> Self {
        Self(value)
    }

    pub(crate) fn as_chrono(&self) -> ChronoTime {
        self.0
    }
}

impl fmt::Display for Time {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.format())
    }
}

impl FromStr for Time {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl Serialize for Time {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.format())
    }
}

impl<'de> Deserialize<'de> for Time {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl Timezone {
    pub fn utc() -> Self {
        Self::Utc
    }

    pub fn parse(value: impl AsRef<str>) -> Result<Self> {
        let value = value.as_ref().trim();
        if value.eq_ignore_ascii_case("UTC") {
            return Ok(Self::Utc);
        }
        if let Ok(value) = value.parse::<Tz>() {
            return Ok(Self::Iana(value));
        }
        if let Ok(value) = value.parse::<FixedOffset>() {
            return Ok(Self::FixedOffset(value));
        }
        Err(Error::message(format!("invalid timezone `{value}`")))
    }

    pub fn as_str(&self) -> String {
        match self {
            Self::Utc => "UTC".to_string(),
            Self::Iana(value) => value.name().to_string(),
            Self::FixedOffset(value) => value.to_string(),
        }
    }

    pub(crate) fn to_fixed_offset_datetime(
        &self,
        value: ChronoDateTime<ChronoUtc>,
    ) -> ChronoDateTime<FixedOffset> {
        match self {
            Self::Utc => value.fixed_offset(),
            Self::Iana(value_tz) => value.with_timezone(value_tz).fixed_offset(),
            Self::FixedOffset(value_offset) => value.with_timezone(value_offset),
        }
    }

    pub(crate) fn local_datetime_to_utc(
        &self,
        value: ChronoNaiveDateTime,
    ) -> Result<ChronoDateTime<ChronoUtc>> {
        let local = match self {
            Self::Utc => ChronoUtc
                .from_local_datetime(&value)
                .single()
                .ok_or_else(|| Error::message("ambiguous or invalid UTC local datetime"))?
                .fixed_offset(),
            Self::Iana(value_tz) => value_tz
                .from_local_datetime(&value)
                .single()
                .ok_or_else(|| {
                    Error::message(format!(
                        "ambiguous or invalid local datetime in timezone `{}`",
                        value_tz.name()
                    ))
                })?
                .fixed_offset(),
            Self::FixedOffset(value_offset) => value_offset
                .from_local_datetime(&value)
                .single()
                .ok_or_else(|| {
                    Error::message(format!(
                        "ambiguous or invalid local datetime in timezone `{value_offset}`"
                    ))
                })?,
        };
        Ok(local.with_timezone(&ChronoUtc))
    }
}

impl fmt::Display for Timezone {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.as_str())
    }
}

impl FromStr for Timezone {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl Serialize for Timezone {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.as_str())
    }
}

impl<'de> Deserialize<'de> for Timezone {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl Clock {
    pub fn new(timezone: Timezone) -> Self {
        Self { timezone }
    }

    pub fn now(&self) -> DateTime {
        DateTime::now()
    }

    pub fn today(&self) -> Date {
        self.now().date_in(&self.timezone)
    }

    pub fn timezone(&self) -> &Timezone {
        &self.timezone
    }
}

fn parse_local_datetime(
    value: &str,
) -> std::result::Result<ChronoNaiveDateTime, chrono::ParseError> {
    ChronoNaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| ChronoNaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f"))
}

fn format_naive_datetime(value: ChronoNaiveDateTime) -> String {
    if value.and_utc().timestamp_subsec_nanos() == 0 {
        value.format("%Y-%m-%dT%H:%M:%S").to_string()
    } else {
        value.format("%Y-%m-%dT%H:%M:%S%.f").to_string()
    }
}

fn parse_time(value: &str) -> std::result::Result<ChronoTime, chrono::ParseError> {
    ChronoTime::parse_from_str(value, "%H:%M:%S%.f")
}

fn format_time(value: ChronoTime) -> String {
    if value.nanosecond() == 0 {
        value.format("%H:%M:%S").to_string()
    } else {
        value.format("%H:%M:%S%.f").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, Date, DateTime, LocalDateTime, Time, Timezone};
    use serde_json::{from_str, to_string};

    #[test]
    fn date_time_round_trips_rfc3339() {
        let value = DateTime::parse("2026-04-11T13:00:00+08:00").unwrap();
        assert_eq!(value.format(), "2026-04-11T05:00:00+00:00");
    }

    #[test]
    fn parse_in_timezone_uses_offset_less_local_values() {
        let tz = Timezone::parse("Asia/Kuala_Lumpur").unwrap();
        let value = DateTime::parse_in_timezone("2026-04-11T13:00:00", &tz).unwrap();
        assert_eq!(value.format(), "2026-04-11T05:00:00+00:00");
        assert_eq!(value.format_in(&tz), "2026-04-11T13:00:00+08:00");
    }

    #[test]
    fn local_date_time_round_trips() {
        let value = LocalDateTime::parse("2026-04-11T13:00:00").unwrap();
        assert_eq!(value.format(), "2026-04-11T13:00:00");
    }

    #[test]
    fn date_round_trips() {
        let value = Date::parse("2026-04-11").unwrap();
        assert_eq!(value.format(), "2026-04-11");
    }

    #[test]
    fn time_round_trips() {
        let value = Time::parse("13:15:00").unwrap();
        assert_eq!(value.format(), "13:15:00");
    }

    #[test]
    fn serde_round_trips_all_datetime_types() {
        let value = DateTime::parse("2026-04-11T13:00:00+08:00").unwrap();
        assert_eq!(
            from_str::<DateTime>(&to_string(&value).unwrap()).unwrap(),
            value
        );

        let local = LocalDateTime::parse("2026-04-11T13:00:00").unwrap();
        assert_eq!(
            from_str::<LocalDateTime>(&to_string(&local).unwrap()).unwrap(),
            local
        );

        let date = Date::parse("2026-04-11").unwrap();
        assert_eq!(from_str::<Date>(&to_string(&date).unwrap()).unwrap(), date);

        let time = Time::parse("13:15:00").unwrap();
        assert_eq!(from_str::<Time>(&to_string(&time).unwrap()).unwrap(), time);
    }

    #[test]
    fn datetime_formats_in_configured_timezone() {
        let timezone = Timezone::parse("Asia/Kuala_Lumpur").unwrap();
        let value = DateTime::parse("2026-04-11T05:00:00Z").unwrap();
        assert_eq!(value.format_in(&timezone), "2026-04-11T13:00:00+08:00");
        assert_eq!(value.date_in(&timezone).format(), "2026-04-11");
        assert_eq!(
            value.local_datetime_in(&timezone).format(),
            "2026-04-11T13:00:00"
        );
    }

    #[test]
    fn clock_uses_timezone_for_today() {
        let clock = Clock::new(Timezone::parse("UTC").unwrap());
        let _ = clock.today();
    }
}
