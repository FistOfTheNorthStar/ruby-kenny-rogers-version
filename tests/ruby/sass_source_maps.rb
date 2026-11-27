# kenny-run-flags: -I "$(ruby -e 'print Gem::Specification.find_by_name("sassc", "2.4.0").gem_dir')/lib"
# Sass source maps, with sassc's options for them: each mapping is printed as the CSS at its
# generated position and the source it points to, so a mapping that is off shows.
require "sassc"
require "json"
require "tmpdir"

B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"

# [[generated line, column, source, original line, column], ...]
def decode(mappings)
  out = []
  src = oline = ocol = 0
  mappings.split(";", -1).each_with_index do |line, gline|
    gcol = 0
    line.split(",").each do |seg|
      values = []
      shift = value = 0
      seg.each_char do |c|
        d = B64.index(c)
        value += (d & 31) << shift
        if d & 32 != 0
          shift += 5
        else
          values << (value & 1 == 1 ? -(value >> 1) : value >> 1)
          shift = value = 0
        end
      end
      gcol += values[0]
      src += values[1]
      oline += values[2]
      ocol += values[3]
      out << [gline, gcol, src, oline, ocol]
    end
  end
  out
end

# The text from a UTF-16 column of a line, up to the end of its first token
def at(text, line, col)
  l = text.lines[line].to_s.chomp.encode("UTF-16LE")
  rest = (l.byteslice(col * 2..) || "").dup.force_encoding("UTF-16LE").encode("UTF-8")
  rest[/\A(@?[^\s{;:,]*)/, 1]
end

def show(css, map, dir)
  sources = map["sources"].map { |s| s == "stdin" ? nil : File.read(File.expand_path(s, dir)) }
  sources = map["sourcesContent"] if map["sourcesContent"]
  decode(map["mappings"]).each do |gl, gc, s, ol, oc|
    puts "  #{gl + 1}:#{gc} #{at(css, gl, gc).inspect} <- #{map["sources"][s]}:#{ol + 1}:#{oc} #{at(sources[s] || STDIN_SRC, ol, oc).inspect}"
  end
end

def render(src, **opts)
  e = SassC::Engine.new(src, style: :expanded, **opts)
  css = e.render
  map = begin
    e.source_map
  rescue SassC::NotRenderedError
    nil
  end
  [css, map]
end

Dir.mktmpdir do |dir|
  dir = File.realpath(dir)
  Dir.chdir(dir) do
    Dir.mkdir("css")
    File.write("css/_part.scss", "$c: red;\n.part {\n  color: $c;\n  a { b: c; }\n}\n")
    main = <<~SCSS
      @import "part";
      // a comment
      /* kept */
      .main {
        width: 10px;
        .inner, .x {
          height: 2px + 3px;
        }
        @media (min-width: 10px) {
          top: 0;
        }
      }
      @supports (display: grid) { .g { display: grid; } }
      @keyframes spin { from { left: 0; } to { left: 1px; } }
    SCSS
    File.write("css/main.scss", main)

    puts "== a map file beside the CSS"
    css, map = render(main, filename: "css/main.scss", source_map_file: "css/main.css.map")
    puts css
    json = JSON.parse(map)
    p json.keys, json["file"], json["sources"]
    show(css, json, "css")

    puts "== a map file elsewhere, an absolute filename"
    css, map = render(main, filename: File.join(dir, "css/main.scss"), source_map_file: "maps/x/out.map")
    puts css.lines.last
    p JSON.parse(map).values_at("file", "sources")

    puts "== omit_source_map_url, source_map_contents (what sprockets asks for, with contents)"
    css, map = render(main, filename: "css/main.scss", source_map_file: "css/main.scss.map",
      omit_source_map_url: true, source_map_contents: true)
    p css.include?("sourceMappingURL")
    json = JSON.parse(map)
    p json.keys, json["sourcesContent"] == [main, File.read("css/_part.scss")]

    puts "== compressed"
    css, map = render(main, filename: "css/main.scss", source_map_file: "css/main.css.map", style: :compressed)
    puts css
    show(css, JSON.parse(map), "css")

    puts "== embedded (sassc-rails' inline_source_maps)"
    css, map = render(main, filename: "css/main.scss", source_map_file: ".", source_map_embed: true, source_map_contents: true)
    data = css[%r{/\*# sourceMappingURL=data:application/json;base64,([A-Za-z0-9+/=]+) \*/\z}, 1]
    p data && data.unpack1("m") == map
    p JSON.parse(map).values_at("file", "sources")
    css, map = render(main, filename: "css/main.scss", source_map_embed: true)
    p css.include?("data:application/json;base64,"), map

    puts "== no filename"
    STDIN_SRC = "a {\n  b: c;\n}\n"
    css, map = render(STDIN_SRC, source_map_file: "out.map")
    puts css
    p JSON.parse(map).values_at("file", "sources")
    show(css, JSON.parse(map), dir)

    puts "== non-ASCII: the @charset line, and columns in UTF-16"
    src = ".é😀 { content: \"😀\"; b: c; }\n"
    File.write("u.scss", src)
    css, map = render(src, filename: "u.scss", source_map_file: "u.map")
    puts css
    show(css, JSON.parse(map), dir)

    puts "== without a source map option"
    css, map = render(main, filename: "css/main.scss")
    p css.include?("sourceMappingURL"), map
  end
end
