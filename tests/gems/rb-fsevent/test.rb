require "bundler/setup"
require "rb-fsevent"

fsevent = FSEvent.new
p fsevent.class, fsevent.paths
fsevent.watch(".", latency: 0.5, no_defer: true) { |dirs| p dirs }
p fsevent.callback.class, fsevent.paths.size
p File.basename(FSEvent.watcher_path)
p FSEvent::VERSION
