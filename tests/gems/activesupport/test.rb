require "bundler/setup"
require "active_support"
require "active_support/core_ext"
require "active_support/cache"

p "person".pluralize, "people".singularize, "active_record/base".camelize, "ActiveRecord::Base".underscore
p "hello world".titleize, "user_id".humanize, "Café Ünïcode".parameterize, "post".classify, "Post".tableize
p 1.ordinalize, 22.ordinalize, 1234567.to_fs(:delimited), 1234.5678.round(2), 12345678.to_fs(:human_size)
p [1, 2, 3].sum, [].blank?, " ".blank?, nil.presence, "x".presence, [1, 2, 3, 4].in_groups_of(3), %w[a b c].to_sentence
p({ a: 1, "b" => 2 }.symbolize_keys, { a: { b: 1 } }.deep_merge(a: { c: 2 }), { a: 1, b: 2 }.except(:a), { a: 1 }.with_indifferent_access[:a])
p 2.days.inspect, (1.hour + 30.minutes).to_i, 3.weeks.in_days, ActiveSupport::Duration.parse("P1Y2M3DT4H").inspect, 90.minutes.iso8601

Time.zone = "Eastern Time (US & Canada)"
t = Time.utc(2024, 3, 10, 6, 30).in_time_zone
p t.to_s, t.zone, (t + 1.hour).to_s, t.beginning_of_day.to_s, (t + 1.day).end_of_month.to_s, t.iso8601
d = Date.new(2024, 1, 31)
p (d + 1.month).to_s, d.next_week.to_s, d.beginning_of_quarter.to_s, d.to_fs(:long), d.yesterday.to_s

events = []
ActiveSupport::Notifications.subscribe("hello.test") { |name, start, finish, id, payload| events << [name, payload] }
r = ActiveSupport::Notifications.instrument("hello.test", n: 1) { 42 }
p r, events

cache = ActiveSupport::Cache::MemoryStore.new
cache.write("k", [1, 2])
p cache.read("k"), cache.fetch("j") { "computed" }, cache.fetch("j") { "again" }, cache.exist?("k"), cache.read_multi("k", "j", "z")
cache.delete("k"); p cache.read("k"), cache.increment("n"), cache.increment("n", 5)

class Account
  include ActiveSupport::Callbacks
  define_callbacks :save
  set_callback :save, :before, -> { log << :before }
  set_callback :save, :after, -> { log << :after }
  attr_accessor :log
  def save; run_callbacks(:save) { log << :save }; end
end
a = Account.new; a.log = []; a.save; p a.log

module Greeting
  extend ActiveSupport::Concern
  included { attr_accessor :name }
  class_methods { def greet_all = "all" }
  def greet = "hi #{name}"
end
class Person; include Greeting; end
pp Person.greet_all, Person.new.tap { _1.name = "x" }.greet
p ActiveSupport::Inflector.transliterate("Ærøskøbing"), "%.2f" % 1.005, "abc".truncate(2), "hello world long".truncate(10)
p ActiveSupport::JSON.encode({ a: [1, nil, 2.5], t: Time.utc(2020, 1, 1) }), ActiveSupport::JSON.decode('{"x":[1,2]}')
p "<b>".html_safe?, ERB::Util.html_escape("<b>&"), ActiveSupport::SafeBuffer.new("a") + "<b>"
p ActiveSupport::NumberHelper.number_to_currency(1234.5), ActiveSupport::NumberHelper.number_to_percentage(12.345, precision: 1)
ms = ActiveSupport::MessageVerifier.new("secret" * 8)
p ms.verified(ms.generate({ "a" => 1 })), ActiveSupport::Deprecation.new("2.0", "Foo").class
p Struct.new(:a).new(1).as_json, 5.clamp(1, 3), 10.in?(1..20), [3, 1, 2].sort.many?, ActiveSupport::OrderedOptions.new.tap { _1.x = 1 }.x
