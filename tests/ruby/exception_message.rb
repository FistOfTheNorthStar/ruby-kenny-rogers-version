# Exception#detailed_message / #full_message formats (CRuby 3.4)
p RuntimeError.new("boom").detailed_message
p RuntimeError.new("").detailed_message
p RuntimeError.new.detailed_message
p StandardError.new("").detailed_message
p RuntimeError.new("line1\nline2").detailed_message
p RuntimeError.new("boom").detailed_message(highlight: true)
p RuntimeError.new("line1\nline2").detailed_message(highlight: true)
p RuntimeError.new("").detailed_message(highlight: true)
p ArgumentError.new("x").detailed_message(highlight: false, extra: 1)
class MyErr < StandardError
  def message = "custom"
end
p MyErr.new.detailed_message
class E2 < StandardError
  def detailed_message(highlight: false, **) = "#{super}!!"
end
e = (raise E2, "hi" rescue $!)
p e.full_message(highlight: false).lines.first.sub(%r{^.*/}, "")
p e.full_message(highlight: false, order: :top).lines.size
e = (raise "a\nb" rescue $!)
p e.full_message(highlight: false).lines.map { _1.sub(/:\d+:/, ":N:").sub(%r{^.*/}, "") }
p e.full_message(highlight: true).lines.first.sub(/:\d+:/, ":N:").sub(%r{^.*/}, "")
p (def hi = 1; self).then { begin; helo; rescue NameError => x; x.message; end }
p (begin; Object.new.instance_eval { zzz }; rescue NameError => x; x.message.sub(/0x\h+/, "XX"); end)
p (begin; nil.zork; rescue NoMethodError => x; x.message; end)
p (begin; Foo; rescue NameError => x; x.message; end)
