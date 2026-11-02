# Library code (found through -I, so compiled with the trailing-Hash keyword rule)
module DepSchema
  module_function

  def object_schema(properties = {}, required: [])
    { properties: properties.keys, required: required }
  end

  def opts(a, o = {}, flag: false) = [a, o, flag]
  def rest(*args, mode: nil) = [args, mode]
  def legacy(a, flag: false) = [a, flag]
  def block_call = yield({ name: "n", extra: 1 })
  def block_kw = yield({ k: 2 })
end
