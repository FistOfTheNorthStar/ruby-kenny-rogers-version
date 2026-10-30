require "bundler/setup"
require "pagy"

class Controller
  include Pagy::Method
  def initialize(params) = @params = params
  def request = { base_url: "http://www.example.com", path: "/items", params: @params }
  public :pagy
end

items = (1..1000).to_a
pagy, records = Controller.new({ "page" => "3" }).pagy(:offset, items)
p [pagy.page, pagy.count, pagy.limit, pagy.last, pagy.from, pagy.to, pagy.previous, pagy.next, pagy.offset]
p records.first(3), records.size
p pagy.send(:series)
puts pagy.series_nav
puts pagy.info_tag
puts pagy.page_url(:next)
puts pagy.previous_tag
p pagy.urls_hash
p pagy.headers_hash
p pagy.data_hash

pagy2, recs2 = Controller.new({ "page" => "50", "limit" => "7" }).pagy(:offset, items, limit: 10, client_limit: 100, slots: 5)
p [pagy2.page, pagy2.limit, pagy2.last, pagy2.send(:series, slots: 5), recs2.first]
puts pagy2.series_nav(:bootstrap)
puts pagy2.series_nav(:bulma)

single = Pagy::Offset.new(count: 5, page: 1)
puts single.info_tag
p single.send(:series)
empty = Pagy::Offset.new(count: 0, page: 1)
puts empty.info_tag

countless, crecs = Controller.new({ "page" => "2" }).pagy(:countless, Pagy::Console::Collection.new(Array(1..45)), limit: 20)
p [countless.page, countless.next, crecs.size]

out = Pagy::Offset.new(count: 100, page: 99, limit: 10, raise_range_error: true) rescue (puts "#{$!.class}: #{$!.message}"; nil)
p out
begin
  Pagy::Offset.new(count: 100, page: 0)
rescue Pagy::OptionError => e
  puts "OptionError: #{e.message}"
end
p Pagy::Offset.new(count: 100, page: 99, limit: 10).then { |x| [x.page, x.in, x.from, x.previous] }
p Pagy::I18n.translate("pagy.item_name", count: 1), Pagy::I18n.translate("pagy.item_name", count: 2)
