require "bundler/setup"
require "tzinfo"

ny = TZInfo::Timezone.get("America/New_York")
t = Time.utc(2024, 3, 10, 6, 59, 59)
p ny.identifier, ny.name, ny.friendly_identifier
p ny.utc_to_local(t).to_s, ny.utc_to_local(t + 1).to_s, ny.to_local(t + 1).to_s
p ny.period_for_utc(t).abbreviation, ny.period_for_utc(t + 1).abbreviation, ny.period_for_utc(t + 1).utc_total_offset
p ny.local_to_utc(Time.utc(2024, 7, 4, 12, 0)).to_s
begin
  ny.local_to_utc(Time.utc(2024, 3, 10, 2, 30))
rescue TZInfo::PeriodNotFound => e
  p e.class
end
begin
  ny.local_to_utc(Time.utc(2024, 11, 3, 1, 30))
rescue TZInfo::AmbiguousTime => e
  p e.class
end
p ny.local_to_utc(Time.utc(2024, 11, 3, 1, 30), true).to_s, ny.local_to_utc(Time.utc(2024, 11, 3, 1, 30), false).to_s
tr = ny.transitions_up_to(Time.utc(2025, 1, 1), Time.utc(2024, 1, 1))
p tr.map { |x| [x.at.to_time.utc.to_s, x.offset.abbreviation.to_s] }
tokyo = TZInfo::Timezone.get("Asia/Tokyo")
p tokyo.utc_to_local(Time.utc(2000, 1, 1)).to_s, tokyo.observed_utc_offset(Time.utc(2000, 1, 1)) rescue p tokyo.current_period.utc_offset
p tokyo.strftime("%Y-%m-%d %H:%M %Z", Time.utc(2020, 6, 1)), ny.strftime("%H:%M %z %Z", Time.utc(2020, 6, 1))
lon = TZInfo::Timezone.get("Europe/London")
p lon.offsets_up_to(Time.utc(2000, 1, 1)).map(&:abbreviation).uniq.sort.first(3)
p TZInfo::Timezone.all_identifiers.include?("Europe/Helsinki"), TZInfo::Country.get("FI").zone_identifiers, TZInfo::Country.get("US").name
begin
  TZInfo::Timezone.get("Nowhere/City")
rescue TZInfo::InvalidTimezoneIdentifier => e
  p e.message
end
p TZInfo::Timezone.get("UTC").utc_to_local(Time.utc(2024, 1, 1)).utc_offset, TZInfo::DataSource.get.class
p TZInfo::Timezone.get("Etc/GMT+5").to_local(Time.utc(2024, 1, 1, 12)).to_s, TZInfo::Timezone.get("US/Eastern").canonical_identifier
