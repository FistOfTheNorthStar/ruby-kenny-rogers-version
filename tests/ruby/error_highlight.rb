# ErrorHighlight is loaded at startup; Rails' debug page calls ErrorHighlight.spot
p defined?(ErrorHighlight)
e = (nil.foo rescue $!)
loc = e.backtrace_locations.first
p ErrorHighlight.spot(e, backtrace_location: loc).nil? || ErrorHighlight.spot(e, backtrace_location: loc).key?(:first_lineno)
