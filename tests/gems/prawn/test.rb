require "bundler/setup"
require "prawn"


Prawn::Fonts::AFM.hide_m17n_warning = true
info = { Title: "Kenny", Author: "Rogers", CreationDate: Time.utc(2026, 1, 2, 3, 4, 5) }
pdf = Prawn::Document.new(page_size: "LETTER", margin: 50, info: info, compress: false)
p pdf.bounds.width, pdf.bounds.height, pdf.page_count, pdf.cursor
pdf.text "Hello, Prawn!", size: 24, style: :bold
pdf.font("Times-Roman") { pdf.text "Line two in Times" }
pdf.move_down 10
pdf.text "Some <b>inline</b> <i>format</i> <color rgb='ff0000'>red</color>", inline_format: true
p pdf.cursor.round(3)
pdf.font("Courier", size: 10) { pdf.text "mono " * 30 }
p pdf.width_of("Hello", size: 12).round(3), pdf.height_of("Hello world " * 40).round(3)
pdf.stroke_color "0000ff"
pdf.stroke_rectangle [0, pdf.cursor], 100, 50
pdf.fill_color "00ff00"
pdf.fill_circle [200, pdf.cursor - 25], 20
pdf.stroke { pdf.line [0, 100], [300, 100] }
pdf.fill_color "000000"
pdf.bounding_box([300, 600], width: 150, height: 100) do
  pdf.text "Inside a box that wraps text across multiple lines nicely", align: :center
  p pdf.bounds.width
  pdf.stroke_bounds
end
pdf.text_box "Boxed text that overflows " * 10, at: [0, 200], width: 120, height: 40, overflow: :shrink_to_fit
leftover = Prawn::Text::Box.new("overflow " * 50, at: [0, 150], width: 80, height: 30, document: pdf).render
p leftover.size
pdf.start_new_page(layout: :landscape)
pdf.text "Page two", align: :right
pdf.column_box([0, pdf.cursor], columns: 2, width: pdf.bounds.width) { pdf.text "col " * 200 }
pdf.repeat(:all) { pdf.draw_text "footer", at: [0, -20] }
pdf.number_pages "<page> of <total>", at: [pdf.bounds.right - 80, 0], width: 80, size: 8
p pdf.page_count, pdf.page_number
pdf.go_to_page(1)
p pdf.page_number
out = pdf.render
p out.encoding, out.start_with?("%PDF-1."), out.end_with?("%%EOF\n")
p out.scan(/\/Type \/Page\b/).size, out.include?("<48656c6c6f2c20507261>"), out.include?("/Title <feff004b0065006e006e0079>") || out.include?("/Title (Kenny)")
p out.scan(%r{/BaseFont /(\S+)}).flatten.uniq.sort
p out.include?("D:20260102030405"), out.scan(/\d+ 0 obj/).size
puts out[/BT.*?ET/m].lines.first(4).join
p out.scan(/^[\d. ]+ (?:SCN|scn|RG|rg)$/).uniq, out.include?("re\n")
begin
  Prawn::Document.new(page_size: "NOPE")
rescue => e
  puts "#{e.class}"
end
begin
  pdf.font("NoSuchFont")
rescue => e
  puts "#{e.class}"
end
p PDF::Core::PageGeometry::SIZES["A4"]
small = Prawn::Document.new(compress: true, info: { CreationDate: Time.utc(2026) })
small.text "compressed"
r = small.render
p r.include?("/FlateDecode"), r.include?("compressed")
