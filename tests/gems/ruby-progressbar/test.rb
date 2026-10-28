require "bundler/setup"
require "ruby-progressbar"
require "stringio"

class FakeTTY < StringIO
  def tty? = true
end

io = FakeTTY.new
bar = ProgressBar.create(title: "Items", total: 10, output: io, length: 50,
                         format: "%t: |%B| %c/%C %p%%", throttle_rate: 0)
4.times { bar.increment }
bar.progress += 2
p bar.progress, bar.to_s
bar.title = "Almost"
bar.log("a log line")
3.times { bar.increment }
bar.finish
p bar.finished?
p io.string.split(/\r/).map(&:rstrip).reject(&:empty?)

io2 = StringIO.new
bar2 = ProgressBar.create(total: 4, output: io2, format: "%w%i|%j%%", length: 30, throttle_rate: 0, progress_mark: "#", remainder_mark: "-")
4.times { bar2.increment }
p io2.string

io3 = FakeTTY.new
bar3 = ProgressBar.create(total: nil, output: io3, length: 30, format: "[%B]", throttle_rate: 0, unknown_progress_animation_steps: ["=--", "-=-", "--="])
3.times { bar3.increment }
bar3.total = 3
bar3.finish
p io3.string.split(/\r/).reject(&:empty?)

bar4 = ProgressBar.create(total: 200, output: StringIO.new, throttle_rate: 0, format: "%c of %C (%P%%) %R")
bar4.progress = 50
p bar4.to_s("%c of %C (%P%%)")
bar4.decrement
p bar4.to_s("%j%% %J%%")
bar4.reset
p bar4.progress
bar4.stop
p bar4.stopped?
begin
  bar4.progress = 500
rescue ProgressBar::InvalidProgressError => e
  p e.message
end
