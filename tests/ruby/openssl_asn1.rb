# OpenSSL::ASN1: decoding (with OpenSSL's errors), encoding with IMPLICIT/EXPLICIT tagging and
# indefinite length, the universal classes, ObjectId names, times, traverse/decode_all
require "openssl"

# (object addresses differ between runs)
def q(*args)
  args.each { |x| puts x.inspect.gsub(/0x\h{6,}/, "0x?") }
  args.size == 1 ? args[0] : args
end

A = OpenSSL::ASN1
q A::Integer.new(1)
q A::BitString.new("ab")
q A.decode("\x02\x01\x05".b)
q OpenSSL::BN.new(5)
q A::Sequence.new([A::Null.new(nil)])
def t
  yield
rescue => e
  puts "#{e.class}: #{e.message}"
end
t { q A.decode("\x02\x01\x01\x00".b) }
t { q A.decode("\x02\x05\x01".b) }
t { q A.decode("\x02\x00".b) }
t { q A.decode("\x30\x03\x02\x01".b) }
t { q A.decode("\x30\x80\x02\x01\x01".b) }
t { q A.decode("\x30\x80\x02\x01\x01\x00\x00".b) }
t { q A.decode("\x01\x02\x00\x00".b) }
t { q A.decode("\x06\x00".b) }
t { q A.decode("\x05\x01\x00".b) }
t { q A.decode("\x17\x05abcde".b) }
t { q A.decode("".b) }
t { q A.decode("\x1f".b) }
t { q A.decode("\x04\x80\x00\x00".b) }
t { q A.decode("\x24\x80\x04\x01a\x00\x00".b) }
t { q A::ObjectId.new("foo").to_der }
t { q A::ObjectId.new("3.5").to_der }
t { q A::Null.new(1).to_der }
t { q A::Boolean.new(nil).to_der }
t { q A::Integer.new("a").to_der }
t { q A::Integer.new(nil).to_der }
t { q A::BitString.new("a").tap { |b| b.unused_bits = 9 }.to_der }
t { q A::Constructive.new([A::Null.new(nil)], 3, :EXPLICIT).to_der }
t { q A::Sequence.new([A::EndOfContent.new, A::Null.new(nil)]).tap { |s| s.indefinite_length = true }.to_der }
t { q A::ASN1Data.new("x", 1, :APPLICATION).tap { |s| s.indefinite_length = true }.to_der }
t { q A::UTCTime.new(Time.utc(2051)).to_der }
t { q A::UTCTime.new(Time.utc(1949)).to_der }
t { q A::GeneralizedTime.new(Time.utc(1949, 1, 2, 3, 4, 5.5)).to_der }
t { q A::UTCTime.new(5).to_der }
t { q A::UTCTime.new("x").to_der }
t { q A::Integer.new(1, nil, nil).to_der }
t { q A::Integer.new(1, nil, :IMPLICIT).to_der }
t { q A::Integer.new(1, 1, "x").to_der }
t { q A::Integer.new(1, 1, :EXPLICIT, "x").to_der }
t { q A::Integer.new(1, 1, :EXPLICIT, :FOO).to_der }
t { q A::ASN1Data.new(1, 1, "x") }
t { q A::Primitive.new("x").to_der }
t { q A::Primitive.new("x", 2).to_der }
t { q A::OctetString.new(5).to_der }
t { q A::OctetString.new("x", 100, :IMPLICIT, :PRIVATE).to_der }
t { q A::OctetString.new("x" * 300, 100, :EXPLICIT, :PRIVATE).to_der.unpack1("H20") }
t { q A.decode(A::OctetString.new("x", 100, :IMPLICIT, :PRIVATE).to_der) }
t { q A.decode("\x02\x01\x80".b).value.to_i }
t { q A.decode("\x03\x02\x07\xff".b) }
t { q A.decode("\x03\x01\x00".b) }
t { q A.decode("\x03\x00".b) }
t { q A.decode("\x17\x0b0001010000Z".b).value }
t { q A.decode("\x17\x0d491231235959Z".b).value }
t { q A.decode("\x17\x0d500101000000Z".b).value }
t { q A.decode("\x18\x0f19990101000000Z".b).value }
t { q A.decode("\x18\x1319990101000000.123Z".b).value }
t { q A.decode("\x06\x03\x2a\x03\x04".b).value }
t { q A.decode("\x06\x03\x55\x04\x03".b) }
t { q A.decode("\x0c\x03abc".b).value.encoding }
t { q A::ObjectId.new("CN").oid, A::ObjectId.new("commonName").sn, A::ObjectId.new("2.5.4.3").ln, A::ObjectId.new("1.2.3").sn }
t { q A::ObjectId.new("1.2.3") == A::ObjectId.new("1.2.3"), A::ObjectId.new("CN") == A::ObjectId.new("2.5.4.3"), A::ObjectId.new("CN") == "CN" }
t { q A::ObjectId.new("nonsense").oid }
t { q A::ObjectId.register("1.2.3.4.5", "kennyTest", "Kenny Test OID"), A::ObjectId.new("1.2.3.4.5").sn, A.decode(A::ObjectId.new("kennyTest").to_der).value }
t { q A::ObjectId.register("1.2.3.4.6", "kennyTest", "Kenny Test OID 2") }
q A::Integer(5)
q A.respond_to?(:Integer), A.respond_to?(:decode), A.singleton_class.public_method_defined?(:Integer)
q(A.const_defined?(:CLASS_TAG_MAP) && A::CLASS_TAG_MAP.size)
A.traverse(A::Sequence.new([A::Integer.new(1), A::Set.new([A::Boolean.new(true)])]).to_der) { |*a| q a }
q A.decode_all("\x02\x01\x01\x05\x00".b)
t { q A.decode_all("\x02\x01\x01\x05".b) }
q A::Sequence.new([A::Integer.new(1)]).each { }.class
q A::Sequence.new([A::Integer.new(1)]).map(&:value)
q A::UTF8String.new("é").to_der
q A::Enumerated.new(-129).to_der, A::Integer.new(OpenSSL::BN.new(255)).to_der, A::Integer.new(-128).to_der, A::Integer.new(0).to_der, A::Integer.new(2**70).to_der
q A.decode(A::Integer.new(-129).to_der).value.to_i
q A::Boolean.new(5).to_der
q A::Set.new([A::Integer.new(1)], 2, :IMPLICIT).to_der
q A::Set.new([A::Integer.new(1)], 2, :EXPLICIT).to_der
s = A::Sequence.new([A::Integer.new(1)], 2, :EXPLICIT); s.indefinite_length = true; p s.to_der
q A.decode(s.to_der)
q A.decode("\x30\x80\x02\x01\x01\x00\x00".b).indefinite_length
q A::EndOfContent.new.to_der
q A.decode("\x00\x00".b)
q A::ASN1Data.new([A::Integer.new(1)], 0, :CONTEXT_SPECIFIC).to_der
q A::Integer.new(1).respond_to?(:indefinite_length=), A::Sequence.new([]).respond_to?(:infinite_length=)
q A::UNIVERSAL_TAG_NAME.frozen?, A::UNIVERSAL_TAG_NAME[0].frozen?
q A.decode(A::Sequence.new([A::ObjectId.new("1.2.840.113549.1.1.11"), A::ObjectId.new("1.3.6.1.5.5.7.48.1"), A::ObjectId.new("2.5.29.17")]).to_der).value.map(&:value)
