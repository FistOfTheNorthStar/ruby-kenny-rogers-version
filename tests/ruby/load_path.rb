# `load` of a relative name looks under the $LOAD_PATH directories (railties loads
# "rails/tasks/framework.rake" so); a file that isn't compiled in runs through the runtime
# compiler (as `eval` does: this test uses it, so the binary has it)
$LOAD_PATH.unshift(File.join(__dir__, "load_path", "lib"))
p load("tasks/hello.rake"), from_rake, $loaded_rake
load "tasks/hello.rake"
p $loaded_rake
begin; load("tasks/missing.rake"); rescue LoadError => e; p [e.class, e.message]; end
p eval("1 + 1")
