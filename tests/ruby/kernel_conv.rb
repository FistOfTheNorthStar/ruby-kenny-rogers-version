["1.5", " 1.5 ", "1_000.5", "1__0", "_1", "1_", ".5", "1.", "1e5", "1E-2", "+3", "-3.0", "0x1A", "0x1.8p1", "inf", "NaN", "Infinity", "", "  ", "1.5x", "1e", "0b11", "0o7", "07", "1\0", "1,5", "\n2\n", "1._5", "1_.5", "1e1_0", "-0x10", "0x", "1.2.3"].each do |s|
  r = begin; Float(s); rescue => e; "#{e.class}: #{e.message}"; end
  p [s, r, Float(s, exception: false)]
end
p Float(nil, exception: false), (Float(nil) rescue $!.message), Float(3), Float(2**70), Float(Rational(1, 2)), (Float(:a) rescue $!.message), Float(:a, exception: false), (Float(Object.new) rescue $!.message)
o = Object.new; def o.to_f = 2.5; p Float(o)
p Integer("12", exception: false), Integer("z", exception: false), Integer(nil, exception: false), Integer("ff", 16, exception: false), Integer("0x1f"), Integer(" 42 "), (Integer("4 2") rescue $!.message), Integer(3.9), Integer("1_000"), (Integer("1__0") rescue $!.message), Integer("0b101"), Integer("-0o17"), Integer("017"), (Integer("09") rescue $!.message), Integer("09", 10)
