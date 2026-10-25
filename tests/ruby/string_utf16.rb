# String#encode to and from UTF-16/32 (Kenny: other methods see the bytes)
a = "hé€😀".encode("UTF-16LE")
p a.bytesize, a.bytes.first(6), a.encoding
b = "hé".encode("UTF-16BE"); p b.bytes
c = "hé".encode("UTF-16"); p c.bytes
d = "hé".encode("UTF-32LE"); p d.bytesize
p a.encode("UTF-8"), a.dup.force_encoding("UTF-16LE").encode("UTF-8")
p "h\x00\xe9\x00".b.force_encoding("UTF-16LE").encode("UTF-8")
p "h\x00\xe9\x00".encode("UTF-8", "UTF-16LE")
p "\xff\xfeh\x00".b.encode("UTF-8", "UTF-16")
p "hé".encode("ISO-8859-1").bytes, "h\xe9".b.encode("UTF-8", "ISO-8859-1")
p "x".encode("UTF-16LE") == "x".encode("UTF-16LE"), "x".encode("UTF-16LE").valid_encoding?
p [0x68, 0xe9].pack("v*").force_encoding("UTF-16LE").encode("UTF-8")
