require "bundler/setup"
require "bigdecimal"
require "action_view"
require "active_support/core_ext/date"
require "tmpdir"
require "fileutils"

dir = Dir.mktmpdir
FileUtils.mkdir_p(File.join(dir, "posts"))
File.write(File.join(dir, "posts/index.html.erb"), <<~ERB)
  <h1><%= title %></h1>
  <ul>
  <% items.each do |item| %>  <%= render "posts/item", item: item %>
  <% end %></ul>
  <%= link_to "Home", "/", class: "nav" %>
ERB
File.write(File.join(dir, "posts/_item.html.erb"), "<li><%= item.upcase %> <%= content_tag(:span, item.size) %></li>")
File.write(File.join(dir, "layout.html.erb"), "<body><%= yield %><%= yield :footer %></body>")
File.write(File.join(dir, "posts/show.html.erb"), "<% content_for :footer, 'F' %><p><%= @post %></p>")

lookup = ActionView::LookupContext.new([dir])
view = ActionView::Base.with_empty_template_cache.new(lookup, { post: "Hello <script>" }, nil)
puts view.render(template: "posts/index", locals: { title: "Posts & more", items: %w[a bb] })
puts view.render(template: "posts/show", layout: "layout")
puts view.render(inline: "<%= 1 + 2 %> <%= raw '<b>' %> <%= '<i>' %>")
puts view.render(collection: %w[x yy], partial: "posts/item", as: :item)

view.instance_eval do
puts tag.div("hi", class: ["a", { b: true, c: false }], data: { id: 1 })
puts number_to_currency(1234.5), number_with_delimiter(1234567), number_to_human(1234567), number_to_human_size(1234567)
puts truncate("A long sentence here", length: 10), pluralize(2, "person"), simple_format("a\n\nb"), excerpt("hello brave new world", "brave", radius: 3)
puts escape_javascript("a'b\n"), strip_tags("<b>bold</b> text"), sanitize("<a href='javascript:x'>x</a><b>ok</b>"), word_wrap("aaa bbb ccc", line_width: 4)
puts time_ago_in_words(Time.now - 3600), distance_of_time_in_words(0, 4000), cycle("odd", "even"), cycle("odd", "even")
puts highlight("ruby is fun", "fun"), auto_discovery_link_tag(:rss, "/feed") rescue puts "adl: #{$!.class}"
puts text_field_tag(:q, "x"), check_box_tag(:ok, "1", true), select_tag(:c, options_for_select([["A", 1], ["B", 2]], 2)), label_tag(:q, "Query")
puts select_date(Date.new(2020, 1, 2), discard_day: true).lines.first(3).join
puts button_tag("Go"), mail_to("a@b.c"), image_tag("/x.png", alt: "x"), javascript_tag("var a = 1;")
puts content_tag(:p, "a <b>", id: "x"), safe_join(["<a>", "b".html_safe], ", "), capture { "cap" }.inspect
end
FileUtils.rm_rf(dir)
