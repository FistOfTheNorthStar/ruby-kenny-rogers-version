require "bundler/setup"
require "aws-partitions"

aws = Aws::Partitions.partition("aws")
p aws.name
r = aws.region("us-east-1")
p r.name, r.description, r.partition_name
p r.services.include?("S3"), r.services.include?("EC2")
p aws.regions.map(&:name).include?("eu-west-1")
p Aws::Partitions.partitions.map(&:name).sort.first(3)
s3 = aws.service("S3")
p s3.name, s3.regionalized?, s3.regions.include?("us-west-2")
p Aws::Partitions::EndpointProvider.resolve("us-west-2", "s3")
p Aws::Partitions::EndpointProvider.resolve("cn-north-1", "ec2")
p Aws::Partitions::EndpointProvider.signing_region("us-gov-west-1", "sts")
p Aws::Partitions::EndpointProvider.dns_suffix_for("cn-north-1")
p Aws::Partitions::EndpointProvider.dns_suffix_for("us-east-1")
p Aws::Partitions.partition("aws-cn").region("cn-north-1").description
begin
  aws.region("mars-1")
rescue ArgumentError => e
  p e.message[0, 40]
end
begin
  Aws::Partitions.partition("nope")
rescue ArgumentError => e
  p e.message[0, 30]
end
