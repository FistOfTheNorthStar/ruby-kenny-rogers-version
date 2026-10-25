# methods / private_methods / instance_methods list what CRuby lists
def hello = 1
o = Object.new
p o.respond_to?(:puts), o.respond_to?(:local_variables), o.respond_to?(:puts, true)
p self.private_methods.include?(:hello), Object.private_instance_methods.include?(:hello), self.methods.include?(:hello)
p RuntimeError.new.respond_to?(:local_variables)
p private_methods.include?(:puts)
o = Object.new
p o.methods.size.between?(45, 60), o.private_methods.size.between?(60, 80), o.public_methods.sort == o.methods.sort, Class.new.instance_methods.size == o.methods.size
p o.methods.include?(:puts), o.private_methods.include?(:puts), o.methods.include?(:dup)
p Class.new.instance_methods.sort == Object.public_instance_methods.concat(Object.protected_instance_methods).uniq.sort
p Object.new.respond_to?(:run), Object.new.respond_to?(:method_missing), Object.new.respond_to?(:singleton_method_added)
