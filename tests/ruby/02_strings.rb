s = "Hello, World"
puts s.length, s.upcase, s.downcase, s.reverse
puts s.include?("World"), s.start_with?("Hell"), s.end_with?("x")
puts s[0], s[0, 5], s[-5..], s[7..11]
puts s.split(", ").inspect
puts "a,b,,c".split(",").inspect
puts "  pad  ".strip + "|"
puts "abc" * 3
puts "hello world".capitalize
puts "hello" + " " + "there"
t = +""
t << "built" << " up"
puts t
puts "snake_case_name".split("_").map(&:capitalize).join
puts "x".ord, 97.chr
puts "hello".sub("l", "L"), "hello".gsub("l", "L")
puts "hello".chars.inspect
puts "line1\nline2".lines.inspect
puts "abc".each_char.to_a.inspect
puts "%s and %s" % ["a", "b"]
puts "42".to_i + 1, "3.5".to_f * 2, "ff".hex
puts "hello world".tr("lo", "01")
puts "aaabbb".squeeze, "hello".count("l")
puts "Ruby".center(10, "*")
puts "abc".succ, "az".succ, "zz".succ
puts :abc.to_s, "abc".to_sym.inspect
puts "été".length, "été".bytesize
sym = :"hello world"
p sym
puts "a-b-c".tr("-", "_")
heredoc = <<~TEXT
  Indented
    more
  back
TEXT
puts heredoc
