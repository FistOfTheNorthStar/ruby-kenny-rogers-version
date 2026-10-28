require "bundler/setup"
require "rainbow"

Rainbow.enabled = true
p Rainbow("red").red
p Rainbow("hi").green.bright.underline
p Rainbow("bg").bg(:yellow).blue
p Rainbow("rgb").color(255, 128, 0)
p Rainbow("hex").color("#00ff88").bg("#112233")
p Rainbow("x11").color(:aqua)
p Rainbow("named").italic.blink.inverse.hide.faint
p Rainbow("cross").cross_out
w = Rainbow.new
w.enabled = false
p w.wrap("plain").red.bold
require "rainbow/refinement"
using Rainbow
p "refined".magenta.bold
p Rainbow.uncolor(Rainbow("stripped").cyan.bold.to_s)
p Rainbow("str").red.is_a?(String), Rainbow("str").red.class
p (Rainbow("a").red + Rainbow("b").blue)
begin
  Rainbow("bad").color(:nonexistent)
rescue ArgumentError => e
  p e.message[0, 60]
end
