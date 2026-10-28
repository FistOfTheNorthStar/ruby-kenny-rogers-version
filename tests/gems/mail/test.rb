require "bundler/setup"
require "mail"

m = Mail.new do
  from    "Alice <alice@example.com>"
  to      ["bob@example.com", "Carol Ç <carol@example.org>"]
  cc      "dave@example.net"
  subject "Hello Kenny — ünïcode"
  date    Time.utc(2024, 1, 2, 3, 4, 5)
  message_id "<abc123@example.com>"
  text_part { body "Plain text body\nline 2" }
  html_part { content_type "text/html; charset=UTF-8"; body "<h1>Hi</h1>" }
end
m.attachments["data.csv"] = { mime_type: "text/csv", content: "a,b\n1,2\n" }
m.attachments["bin.dat"] = "\x00\x01\x02binary".b
m.header["X-Custom"] = "yes"
p m.from, m.to, m.cc, m[:from].display_names, m.subject, m.date.to_s, m.message_id
p m.multipart?, m.parts.size, m.attachments.map(&:filename), m.attachments.map(&:mime_type)
p m.text_part.body.decoded, m.html_part.content_type, m["X-Custom"].value
s = m.to_s.gsub(/--==_mimepart_\w+/, "--BOUNDARY").gsub(/boundary="?[^";\r\n]+"?/, "boundary=B").gsub(/Content-ID: <[^>]+>/, "Content-ID: <ID>")
puts s.lines.grep_v(/^Mime-Version|^\r?$/).first(12).join
p m[:subject].encoded

parsed = Mail.read_from_string(m.to_s)
p parsed.subject, parsed.from, parsed.to, parsed.parts.size, parsed.attachments.map(&:filename)
p parsed.attachments.first.decoded, parsed.attachments.last.decoded.bytes.first(3)
p parsed.text_part.decoded, parsed.html_part.decoded

raw = <<~MAIL
  From: =?UTF-8?B?SsO8cmdlbg==?= <j@example.com>
  To: x@example.com
  Subject: =?ISO-8859-1?Q?Gr=FC=DFe?=
  Content-Type: text/plain; charset=UTF-8
  Content-Transfer-Encoding: quoted-printable

  Caf=C3=A9 au lait=
   soft break
MAIL
r = Mail.new(raw)
p r[:from].display_names, r.subject, r.body.decoded, r.charset, r.content_transfer_encoding
a = Mail::Address.new('"Smith, John" <john.smith+tag@Example.COM>')
p a.display_name, a.local, a.domain, a.address, a.format
p Mail::Encodings::Base64.encode("hello"), Mail::Encodings::QuotedPrintable.encode("héllo=")
p Mail::Utilities.respond_to?(:blank?), Mail::Header.new("A: 1\r\nB: 2").fields.map(&:name)
Mail.defaults { delivery_method :test }
m.delivery_method :test
m.deliver
Mail.new(from: "a@b.c", to: "d@e.f", subject: "second", body: "x").deliver
p Mail::TestMailer.deliveries.size, Mail::TestMailer.deliveries.first.subject
