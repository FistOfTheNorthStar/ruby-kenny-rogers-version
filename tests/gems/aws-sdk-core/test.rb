require "bundler/setup"
require "aws-sdk-core"

creds = Aws::Credentials.new("AKID", "SECRET", "TOKEN")
p creds.set?, creds.access_key_id
sts = Aws::STS::Client.new(region: "us-east-1", credentials: creds, stub_responses: true)
p sts.config.region, sts.config.endpoint.to_s
sts.stub_responses(:get_caller_identity, account: "123456789012", arn: "arn:aws:iam::123456789012:user/kenny", user_id: "AIDA")
r = sts.get_caller_identity
p r.account, r.arn, r.class.name, r.successful?
p r.to_h
p sts.api_requests.size, sts.api_requests.first[:operation_name]
req = r.context.http_request
p req.http_method, req.endpoint.to_s
p req.body.read.split("&").sort
p req.headers["authorization"]&.start_with?("AWS4-HMAC-SHA256 Credential=AKID/")

sts.stub_responses(:assume_role, { credentials: { access_key_id: "ASIA", secret_access_key: "s", session_token: "t", expiration: Time.utc(2030, 1, 1) } })
ar = sts.assume_role(role_arn: "arn:aws:iam::1:role/r", role_session_name: "x")
p ar.credentials.access_key_id, ar.credentials.expiration

sts.stub_responses(:get_session_token, "ExpiredTokenException")
begin
  sts.get_session_token
rescue Aws::STS::Errors::ExpiredTokenException => e
  p e.class, e.code, e.is_a?(Aws::Errors::ServiceError)
end
begin
  sts.assume_role(role_arn: 1)
rescue ArgumentError => e
  p e.message.lines.first.strip
end
data = sts.stub_data(:get_caller_identity)
p data.to_h.keys.sort
p Aws::Structure.new(:a, :b).new(a: 1).to_h
p Aws::Json.dump({ "a" => [1, 2] }), Aws::Json.load('{"x":1}')
p Aws::Xml::Parser.new(Seahorse::Model::Shapes::ShapeRef.new(shape: Aws::STS::ClientApi::GetCallerIdentityResponse)).parse(
  "<GetCallerIdentityResult><Account>42</Account><Arn>a</Arn></GetCallerIdentityResult>"
).to_h
p Aws.shared_config.class
p Aws::Partitions.partition("aws").name
