require "bundler/setup"
require "aws-sdk-kms"

kms = Aws::KMS::Client.new(region: "us-west-2", credentials: Aws::Credentials.new("AK", "SK"), stub_responses: true)
kms.stub_responses(:encrypt, ->(ctx) { { ciphertext_blob: ctx.params[:plaintext].reverse, key_id: ctx.params[:key_id] } })
kms.stub_responses(:decrypt, ->(ctx) { { plaintext: ctx.params[:ciphertext_blob].reverse, key_id: "arn:aws:kms:us-west-2:1:key/k" } })
enc = kms.encrypt(key_id: "alias/app", plaintext: "secret data", encryption_context: { "purpose" => "test" })
p enc.ciphertext_blob, enc.key_id
dec = kms.decrypt(ciphertext_blob: enc.ciphertext_blob)
p dec.plaintext, dec.key_id
body = JSON.parse(enc.context.http_request.body.read)
p body.sort.to_h
p enc.context.http_request.headers["x-amz-target"], enc.context.http_request.headers["content-type"]
kms.stub_responses(:list_keys, keys: [{ key_id: "k1", key_arn: "arn:1" }, { key_id: "k2", key_arn: "arn:2" }], truncated: false)
p kms.list_keys.keys.map(&:key_id)
kms.stub_responses(:generate_data_key, plaintext: "p" * 32, ciphertext_blob: "c" * 8, key_id: "k1")
dk = kms.generate_data_key(key_id: "k1", key_spec: "AES_256")
p dk.plaintext.bytesize, dk.ciphertext_blob
kms.stub_responses(:describe_key, "NotFoundException")
begin
  kms.describe_key(key_id: "nope")
rescue Aws::KMS::Errors::NotFoundException => e
  p e.class, e.code
end
kms.stub_responses(:describe_key, key_metadata: { key_id: "k1", key_state: "Enabled", key_usage: "ENCRYPT_DECRYPT", creation_date: Time.utc(2020, 1, 1) })
md = kms.describe_key(key_id: "k1").key_metadata
p md.key_id, md.key_state, md.creation_date.utc
p Aws::KMS::Client.api.operation_names.include?(:sign)
