require "bundler/setup"
require "chronic"

ENV["TZ"] = "UTC"
Chronic.time_class = Time
now = Time.utc(2026, 3, 18, 14, 30, 0) # a Wednesday
[
  "tomorrow", "yesterday at 4pm", "today 7am", "next tuesday", "last friday noon",
  "3 days from now", "2 weeks ago", "in 5 hours", "next month", "last year",
  "monday 9:15", "this afternoon", "tonight", "may 27th", "27 May 2027 at 9pm",
  "1979-05-27 05:00:00", "5/27/2025", "27/5/2025", "Jan 1st at midnight", "the 3rd of april",
  "third wednesday in november", "noon", "6 in the morning", "sat 7 in the evening",
  "2 months ago", "1 week hence", "afternoon yesterday", "thursday last week",
  "December 25", "4th day last week", "quarter past six", "end of month",
].each do |s|
  r = Chronic.parse(s, now: now)
  puts "#{s.ljust(28)} => #{r.inspect}"
end
span = Chronic.parse("next week", now: now, guess: false)
p span.class, span.begin, span.end, span.width
p Chronic.parse("may", now: now, guess: false)
p Chronic.parse("today", now: now, guess: :begin), Chronic.parse("today", now: now, guess: :end)
p Chronic.parse("2/3/2026", now: now, endian_precedence: [:little, :middle])
p Chronic.parse("2/3/2026", now: now)
p Chronic.parse("next spring", now: now, context: :past)
p Chronic.parse("monday", now: now, context: :past)
p Chronic.parse("5", now: now, hours24: true), Chronic.parse("5:00", now: now, ambiguous_time_range: :none)
p Chronic.parse("gibberish nonsense", now: now)
p Chronic.parse("", now: now)
p Chronic::Numerizer.numerize("forty two apples and one hundred twenty pears")
p Chronic::Numerizer.numerize("third and twenty-first")
begin
  p Chronic.parse("tomorrow", now: now, bogus: 1)
rescue ArgumentError => e
  puts "ArgumentError: #{e.message}"
end
p Chronic.parse("tomorrow at 3pm", now: Time.utc(2026, 12, 31, 23, 0))
