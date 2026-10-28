require "bundler/setup"
require "action_mailer"

ActionMailer::Base.delivery_method = :test
ActionMailer::Base.view_paths = []
class UserMailer < ActionMailer::Base
  default from: "noreply@example.com"
  layout false
  def welcome(name, email)
    @name = name
    attachments["notes.txt"] = "some notes"
    headers["X-Campaign"] = "spring"
    mail(to: email, subject: "Welcome #{name}", date: Time.utc(2024, 1, 2, 3, 4, 5), message_id: "<fixed@example.com>") do |format|
      format.text { render plain: "Hi #{@name}, welcome!" }
      format.html { render html: "<p>Hi <b>#{ERB::Util.h(@name)}</b></p>".html_safe }
    end
  end
  def plain(to)
    mail(to: to, cc: ["c@example.com"], subject: "Plain ünïcode", body: "Just text", content_type: "text/plain",
         date: Time.utc(2024, 1, 2), message_id: "<p@example.com>")
  end
end

class Interceptor
  def self.delivering_email(m) = m.subject = "[i] #{m.subject}"
end
ActionMailer::Base.register_interceptor(Interceptor)

m = UserMailer.welcome("Kenny <K>", "kenny@example.com")
p m.class, m.to, m.from, m.subject, m.multipart?, m.parts.map(&:content_type).map { _1.split(";").first }
p m.attachments.map(&:filename), m["X-Campaign"].value, m.text_part.body.decoded, m.html_part.body.decoded
p ActionMailer::Base.deliveries.size
m.deliver_now
p ActionMailer::Base.deliveries.size, ActionMailer::Base.deliveries.last.subject
pl = UserMailer.plain("x@example.com")
pl.deliver_now
enc = pl.message.encoded
puts enc.lines.reject { |l| l.start_with?("Mime-Version") }.join.gsub("\r\n", "\n")
p UserMailer.action_methods.sort, ActionMailer::Base.deliveries.map(&:to)
mail = Mail.new { from "a@b.c"; to "d@e.f"; subject "raw"; body "b" }
p mail.to, Mail::Address.new("Kenny <k@x.y>").display_name, Mail.new("Subject: hi\r\n\r\nbody").subject
