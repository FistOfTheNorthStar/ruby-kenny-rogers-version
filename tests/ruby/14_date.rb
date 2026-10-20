require "date"

# --- construction & accessors ---
d = Date.new(2024, 10, 19)
p d
puts d, d.year, d.mon, d.month, d.mday, d.day, d.wday, d.yday, d.jd, d.mjd
puts d.cwyear, d.cweek, d.cwday
p Date.new, Date.new(2024), Date.new(2024, -1, -1), Date.civil(2024, 2, -1)
p Date.jd(2460603), Date.ordinal(2024, 293), Date.ordinal(2024, -1), Date.commercial(2024, 42, 6)
p Date.new(1582, 10, 15), Date.new(1582, 10, 4) + 1, Date.new(-5, 1, 1).to_s
p d.saturday?, d.sunday?, d.leap?, Date.new(2023).leap?, Date.new(1500).leap?
p Date.leap?(2000), Date.leap?(1900), Date.gregorian_leap?(2024), Date.julian_leap?(1900)
p Date.valid_date?(2024, 2, 29), Date.valid_date?(2023, 2, 29), Date.valid_civil?(2024, 13, 1)
p Date.valid_ordinal?(2024, 366), Date.valid_ordinal?(2023, 366), Date.valid_commercial?(2020, 53, 1), Date.valid_commercial?(2021, 53, 1)
p Date::MONTHNAMES[10], Date::ABBR_MONTHNAMES[1], Date::DAYNAMES[0], Date::ABBR_DAYNAMES, Date::ITALY, Date::ENGLAND
p Date::Error.superclass
begin
  Date.new(2024, 2, 30)
rescue Date::Error => e
  p [e.class, e.message]
end

# --- arithmetic ---
p d + 1, d - 1, d + 365, d >> 1, d << 1, d >> 12
p Date.new(2024, 1, 31) >> 1, Date.new(2024, 3, 31) << 1, Date.new(2024, 2, 29).next_year, Date.new(2024, 2, 29).prev_year
p d.next_day, d.prev_day(3), d.next_month(3), d.prev_month, d.succ, d.next
p (d - Date.new(2024, 1, 1)).to_i, (Date.new(2024, 1, 1) - d).to_i
p d + 1.5
begin
  d + "x"
rescue TypeError => e
  p e.message
end

# --- comparison / hashing ---
p d == Date.new(2024, 10, 19), d != Date.new(2024, 10, 20), d < Date.new(2025, 1, 1), d <=> Date.new(2020, 1, 1)
p d <=> "x", d == "2024-10-19", d === DateTime.new(2024, 10, 19, 23), d == DateTime.new(2024, 10, 19)
p d.eql?(Date.new(2024, 10, 19)), d.hash == Date.new(2024, 10, 19).hash
p d.between?(Date.new(2024, 1, 1), Date.new(2024, 12, 31)), d.clamp(Date.new(2025, 1, 1), Date.new(2025, 2, 1)).to_s
h = { Date.new(2024, 1, 1) => :new_year }
p h[Date.new(2024, 1, 1)], h.key?(Date.new(2024, 1, 2))
p [Date.new(2024, 3, 1), Date.new(2023, 1, 1), Date.new(2024, 1, 1)].sort.map(&:to_s)
p [Date.new(2024, 3, 1), Date.new(2023, 1, 1)].max.to_s, [d, d, d + 1].uniq.size

# --- iteration ---
p d.step(Date.new(2024, 10, 25), 3).map(&:to_s)
p d.step(Date.new(2024, 10, 1), -7).map(&:day)
p Date.new(2024, 1, 1).upto(Date.new(2024, 1, 3)).map(&:day), Date.new(2024, 1, 3).downto(Date.new(2024, 1, 1)).map(&:day)
p (Date.new(2024, 1, 30)..Date.new(2024, 2, 2)).map(&:to_s)
p (Date.new(2024, 1, 1)...Date.new(2024, 1, 4)).to_a.size
p (Date.new(2024, 1, 1)..Date.new(2024, 1, 10)).include?(Date.new(2024, 1, 5))
months = Hash.new(0)
(Date.new(2024, 1, 1)..Date.new(2024, 12, 31)).each { |x| months[x.strftime("%b")] += 1 }
p months

# --- formatting ---
p d.to_s, d.inspect, d.iso8601, d.xmlschema, d.rfc3339, d.rfc2822, d.httpdate, d.jisx0301, d.asctime, d.ctime
p d.strftime, d.strftime("%Y/%m/%d %a %A %b %B %j %U %W %G-W%V-%u %C %y %e %-m/%-d")
p d.strftime("%c|%x|%X|%D|%F|%T|%+|%v|%s|%Q|%L|%N|%3N|%z|%:z|%::z|%Z|%^a|%#B|%10A|%-d|%_m|%%|%q")
p Date.new(5, 1, 1).strftime("%Y %C %y"), Date.new(-5, 1, 1).strftime("%Y %C %y")
p Date.new(1990, 1, 8).jisx0301, Date.new(2019, 4, 30).jisx0301, Date.new(2019, 5, 1).jisx0301

# --- DateTime ---
dt = DateTime.new(2024, 10, 19, 12, 30, 45, "+09:00")
p dt, dt.class, dt.class.superclass
puts dt, dt.hour, dt.min, dt.minute, dt.sec, dt.second, dt.zone
p (dt.offset * 24).to_i, (dt.sec_fraction * 1000).round
p dt.new_offset(0), dt.new_offset("-05:00").to_s, dt.new_offset(-0.25).to_s
p DateTime.new(2024, 10, 19, 1, 2, 3.25).iso8601(3), dt.iso8601, dt.xmlschema, dt.rfc3339, dt.jisx0301
p dt.strftime, dt.strftime("%I:%M:%S %p %P %l %k %H")
p DateTime.new(2024, 10, 19, 24, 0, 0), DateTime.new(2024, 10, 19, -1, -1, -1)
p DateTime.new(2024, 10, 19, 12, 30, 0, 0.375).zone, DateTime.jd(2460603, 6).to_s
p ((dt - DateTime.new(2024, 10, 19, 0, 0, 0)) * 24 * 60).round
p (dt + 0.5).to_s, (dt >> 1).to_s, dt.to_date, dt.to_date.class, d.to_datetime
p DateTime.new(2024, 10, 19, 23, 0, 0, "-05:00").to_date.to_s
p dt == DateTime.new(2024, 10, 19, 3, 30, 45), dt.new_offset(0).hash == dt.hash

# --- parsing ---
p Date._parse("2024-10-19")
p Date._parse("Sat, 19 Oct 2024 12:30:00 +0000")
p Date._parse("Oct 19 2024"), Date._parse("19 October 2024 3pm EST"), Date._parse("2024/10/19")
p Date._parse("10/19/2024"), Date._parse("20241019T123000Z"), Date._parse("H31.04.30")
p Date._parse("Saturday, October 19, 2024"), Date._parse("19th Oct"), Date._parse("2024-W42-6")
p Date._parse("Tue May 07 2024 12:00:00 GMT+0200 (CEST)"), Date._parse("12:30"), Date._parse("20241019")
p Date._parse("19-Oct-2024"), Date._parse("2024.10.19"), Date._parse("Oct 19, 99"), Date._parse("Oct 19, 99", false)
p Date._parse("5 BC Jan 1")
p Date.parse("2024-10-19"), Date.parse("Oct 19 2024"), Date.parse("19 Oct 2024"), Date.parse("Sat, 19 Oct 2024")
p Date.parse("2024-10-19T23:00:00-05:00"), Date.parse("20241019")
p DateTime.parse("2024-10-19T12:30:45+09:00"), DateTime.parse("Sat, 19 Oct 2024 12:30:00 -0700").to_s
p (DateTime.parse("2024-10-19T12:30:45.123+09:00").sec_fraction * 1000).round
%w[foo 2024-02-30 10/19/2024].each do |s|
  Date.parse(s)
rescue Date::Error => e
  p [s, e.message]
end
p Date._iso8601("2024-10-19"), Date._iso8601("20241019T123045Z"), Date._iso8601("12:30"), Date._iso8601("2024-293")
p Date.iso8601("2024-10-19"), DateTime.iso8601("2024-10-19T12:30:45+09:00").new_offset("-05:00").to_s
p Date._rfc3339("2024-10-19T12:30:00+09:00"), DateTime.rfc3339("2024-10-19T12:30:00+09:00")
p Date._xmlschema("2024-10-19"), Date.xmlschema("2024-10-19")
p Date._rfc2822("Sat, 19 Oct 2024 12:30:00 +0900"), Date.rfc2822("Sat, 19 Oct 2024 12:30:00 +0900")
p Date._httpdate("Sat, 19 Oct 2024 00:00:00 GMT"), DateTime.httpdate("Sat, 19 Oct 2024 12:30:00 GMT")
p Date.jisx0301("R06.10.19")

# --- strptime ---
p Date._strptime("2024-10-19", "%Y-%m-%d"), Date._strptime("2024-10-19x", "%Y-%m-%d"), Date._strptime("x", "%Y")
p Date._strptime("24 3pm", "%y %I%p"), Date._strptime("1729296000", "%s")
p Date.strptime("19/10/2024", "%d/%m/%Y"), Date.strptime("2024-10-19"), Date.strptime("2024 42 6", "%G %V %u")
p Date.strptime("2024-293", "%Y-%j"), Date.strptime("Saturday October 19 2024", "%A %B %d %Y")
p DateTime.strptime("2024-10-19 12:30 PM", "%Y-%m-%d %I:%M %p"), DateTime.strptime("1729296000", "%s")
p DateTime.strptime("19/Oct/2024:12:30:45 +0200", "%d/%b/%Y:%H:%M:%S %z").to_s
begin
  Date.strptime("2024-13-01", "%Y-%m-%d")
rescue Date::Error => e
  p e.message
end

# --- conversions & misc ---
p Time.at(1729296000).to_date.class, Time.at(1729296000).to_datetime.class
t = Date.new(2024, 10, 19).to_time
p t.class, t.year, t.month, t.day, t.hour
p DateTime.new(2024, 10, 19, 12, 30, 0).to_time.to_i
p Time.at(1729341000).to_datetime.new_offset(0).to_s
p Date.today.class, DateTime.now.class, Date.today == Time.now.to_date
p d.infinite?, Date::Infinity.new > 3, d.frozen?, d.dup == d, d.to_date.equal?(d)
case d
in { year: 2024, month: m, day: } then p [m, day]
end
p dt.deconstruct_keys([:hour, :zone])
class Date
  def plus_with_duration(other)
    other.is_a?(Hash) ? self + other[:days] : plus_without_duration(other)
  end
  alias_method :plus_without_duration, :+
  alias_method :+, :plus_with_duration
end
p (d + { days: 3 }).to_s, (d + 1).to_s
class MyDate < Date; end
p MyDate.new(2024, 1, 2).class, (MyDate.new(2024, 1, 2) + 1).class, MyDate.parse("2024-01-02").class
