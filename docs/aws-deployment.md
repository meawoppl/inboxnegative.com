# AWS Deployment Guide for InboxNegative

This guide documents the complete AWS infrastructure setup for the InboxNegative application, including ECS deployment, DNS configuration, and SSL/HTTPS setup.

## Table of Contents

1. [Infrastructure Overview](#infrastructure-overview)
2. [Prerequisites](#prerequisites)
3. [AWS Infrastructure Setup](#aws-infrastructure-setup)
4. [Docker Image Build and Push](#docker-image-build-and-push)
5. [ECS Deployment](#ecs-deployment)
6. [DNS Configuration](#dns-configuration)
7. [SSL/HTTPS Setup](#sslhttps-setup)
8. [Troubleshooting](#troubleshooting)
9. [Monitoring and Logs](#monitoring-and-logs)

## Infrastructure Overview

The InboxNegative application runs on the following AWS infrastructure:

- **Compute**: EC2 instance (t2.micro) running ECS agent
- **Container Orchestration**: ECS (Elastic Container Service) with EC2 launch type
- **Container Registry**: ECR (Elastic Container Registry)
- **DNS**: Route53 hosted zone
- **Database**: Neon PostgreSQL (external, managed)
- **Web Server**: Nginx reverse proxy on EC2 instance
- **SSL**: Let's Encrypt certificates via acme.sh

### Architecture Diagram

```
Internet
    │
    ├─> Route53 DNS (inboxnegative.com)
    │       │
    │       └─> EC2 Instance (18.237.84.151)
    │               │
    │               ├─> Nginx (port 80/443)
    │               │       │
    │               │       └─> Docker Container (port 8080)
    │               │               │
    │               │               ├─> HTTP Server
    │               │               └─> SMTP Server (port 2525)
    │               │
    │               └─> ECS Agent
    │
    └─> Neon PostgreSQL Database
```

## Prerequisites

Before starting the deployment, ensure you have:

1. **AWS CLI** installed and configured with appropriate credentials
2. **Docker** installed for building images
3. **AWS Account** with the following permissions:
   - EC2 full access
   - ECS full access
   - ECR full access
   - Route53 hosted zone management
   - CloudWatch Logs access
4. **Domain Name** pointed to Route53 (inboxnegative.com in this case)
5. **SSH Key Pair** for EC2 access (inboxnull.pem)
6. **Neon Database** instance with connection string

## AWS Infrastructure Setup

### 1. EC2 Instance Setup

The application runs on a single EC2 instance with ECS agent installed.

#### Instance Details
- **Instance Type**: t2.micro
- **AMI**: Amazon Linux 2023 with ECS optimized
- **Instance ID**: i-044518e959d9a7a7b
- **Public IP**: 18.237.84.151
- **Security Groups**: Allow ports 22, 80, 443, 2525, 8080

#### IAM Role Configuration

The EC2 instance requires an IAM instance profile with the following permissions:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "ecs:CreateCluster",
        "ecs:DeregisterContainerInstance",
        "ecs:DiscoverPollEndpoint",
        "ecs:Poll",
        "ecs:RegisterContainerInstance",
        "ecs:StartTelemetrySession",
        "ecs:Submit*",
        "ecr:GetAuthorizationToken",
        "ecr:BatchCheckLayerAvailability",
        "ecr:GetDownloadUrlForLayer",
        "ecr:BatchGetImage",
        "logs:CreateLogStream",
        "logs:PutLogEvents"
      ],
      "Resource": "*"
    }
  ]
}
```

**Important**: If the instance profile becomes disassociated (e.g., after account suspension), reassociate it:

```bash
# Find the association ID
aws ec2 describe-iam-instance-profile-associations

# Disassociate if needed
aws ec2 disassociate-iam-instance-profile --association-id <association-id>

# Reassociate
aws ec2 associate-iam-instance-profile \
  --instance-id i-044518e959d9a7a7b \
  --iam-instance-profile Name=InboxNullECSAccess
```

#### Metadata Service Configuration

Increase the metadata hop limit to allow Docker containers to access instance metadata:

```bash
aws ec2 modify-instance-metadata-options \
  --instance-id i-044518e959d9a7a7b \
  --http-put-response-hop-limit 2
```

### 2. ECS Cluster Setup

Create the ECS cluster:

```bash
aws ecs create-cluster --cluster-name inboxnegative-cluster
```

Register the EC2 instance with the cluster:

```bash
# SSH into the EC2 instance
ssh -i inboxnull.pem ec2-user@18.237.84.151

# Edit ECS configuration
sudo nano /etc/ecs/ecs.config

# Add cluster name
ECS_CLUSTER=inboxnegative-cluster

# Restart ECS agent
sudo stop ecs
sudo start ecs
```

**Troubleshooting**: If the agent database has old cluster information:

```bash
sudo stop ecs
sudo rm -f /var/lib/ecs/data/agent.db
sudo start ecs
```

### 3. ECR Repository Setup

Create the container registry:

```bash
aws ecr create-repository --repository-name inboxnegative --region us-west-2
```

This creates a repository at: `877983347039.dkr.ecr.us-west-2.amazonaws.com/inboxnegative`

### 4. CloudWatch Logs Setup

Create a log group for ECS task logs:

```bash
aws logs create-log-group --log-group-name /ecs/inboxnegative
```

## Docker Image Build and Push

### Build Process

The application uses a multi-stage Dockerfile:

1. **Builder stage**: Debian Bullseye with Rust toolchain
2. **Runtime stage**: Debian Bullseye Slim with minimal dependencies

Environment variables are passed via ECS task definition, not baked into the image.

### Build and Push Script

The `push-to-ecr.sh` script handles the complete build and deployment:

```bash
#!/bin/bash

# Authenticate with ECR
aws ecr get-login-password --region us-west-2 | \
  docker login --username AWS --password-stdin \
  877983347039.dkr.ecr.us-west-2.amazonaws.com

# Build the Docker image
docker build -t inboxnegative:latest .

# Tag for ECR
docker tag inboxnegative:latest \
  877983347039.dkr.ecr.us-west-2.amazonaws.com/inboxnegative:latest

# Push to ECR
docker push 877983347039.dkr.ecr.us-west-2.amazonaws.com/inboxnegative:latest

# Register new task definition
aws ecs register-task-definition \
  --cli-input-json file://devops/task-definition.json

# Update the service
aws ecs update-service \
  --cluster inboxnegative-cluster \
  --service inboxnegative-service \
  --task-definition inboxnegative-task
```

Run the deployment:

```bash
./push-to-ecr.sh
```

## ECS Deployment

### Task Definition

The ECS task definition is stored in `devops/task-definition.json`:

```json
{
    "family": "inboxnegative-task",
    "networkMode": "bridge",
    "containerDefinitions": [
        {
            "name": "inboxnegative",
            "image": "877983347039.dkr.ecr.us-west-2.amazonaws.com/inboxnegative:latest",
            "memory": 128,
            "cpu": 128,
            "essential": true,
            "portMappings": [
                {
                    "containerPort": 2525,
                    "hostPort": 2525,
                    "protocol": "tcp"
                },
                {
                    "containerPort": 8080,
                    "hostPort": 8080,
                    "protocol": "tcp"
                }
            ],
            "environment": [
                {
                    "name": "DATABASE_URL",
                    "value": "postgresql://..."
                },
                {
                    "name": "INBOXNULL_HOSTNAME",
                    "value": "https://inboxnegative.com"
                },
                {
                    "name": "GOOGLE_CLIENT_ID",
                    "value": "..."
                },
                {
                    "name": "GOOGLE_CLIENT_SECRET",
                    "value": "..."
                },
                {
                    "name": "RUST_LOG",
                    "value": "info"
                }
            ],
            "logConfiguration": {
                "logDriver": "awslogs",
                "options": {
                    "awslogs-group": "/ecs/inboxnegative",
                    "awslogs-region": "us-west-2",
                    "awslogs-stream-prefix": "ecs"
                }
            }
        }
    ],
    "requiresCompatibilities": ["EC2"],
    "cpu": "128",
    "memory": "256"
}
```

### Environment Variables

Critical environment variables that must be set:

| Variable | Purpose | Example |
|----------|---------|---------|
| `DATABASE_URL` | PostgreSQL connection string | `postgresql://user:pass@host/db?sslmode=require` |
| `INBOXNULL_HOSTNAME` | Public URL of the application | `https://inboxnegative.com` |
| `GOOGLE_CLIENT_ID` | OAuth client ID | `780628753905-...` |
| `GOOGLE_CLIENT_SECRET` | OAuth client secret | `GOCSPX-...` |
| `RUST_LOG` | Logging level | `info` |

### Service Creation

Create the ECS service with deployment configuration to prevent port conflicts:

```bash
aws ecs create-service \
  --cluster inboxnegative-cluster \
  --service-name inboxnegative-service \
  --task-definition inboxnegative-task \
  --desired-count 1 \
  --launch-type EC2 \
  --deployment-configuration "minimumHealthyPercent=0,maximumPercent=200"
```

**Important Deployment Configuration**:
- `minimumHealthyPercent=0`: Allows ECS to stop the old task before starting the new one
- `maximumPercent=200`: Allows up to 2x the desired count during deployments
- This configuration prevents port binding conflicts on single-instance deployments
- Brief service outage occurs during deployments (old task stops, then new task starts)
- Eliminates need for manual intervention during deployments

If you need to update the deployment configuration on an existing service:

```bash
aws ecs update-service \
  --cluster inboxnegative-cluster \
  --service-name inboxnegative-service \
  --deployment-configuration "minimumHealthyPercent=0,maximumPercent=200"
```

### Memory Considerations

The application uses minimal memory (~128MB). The task definition allocates:
- **Container Memory**: 128 MB
- **Task Memory**: 256 MB
- **Container CPU**: 128 units
- **Task CPU**: 128 units

This allows multiple tasks to run on a t2.micro instance if needed.

**Troubleshooting Memory Issues**: If deployments fail with "insufficient memory", manually stop the old task:

```bash
# List tasks
aws ecs list-tasks --cluster inboxnegative-cluster

# Stop old task
aws ecs stop-task \
  --cluster inboxnegative-cluster \
  --task <task-id> \
  --reason "Freeing memory for new deployment"
```

## DNS Configuration

### Route53 Setup

The domain `inboxnegative.com` is managed in Route53 with hosted zone ID: `Z03947011ZQ3P0KMABYWB`

#### A Records

Two A records point to the EC2 instance:

```bash
# Main domain
aws route53 change-resource-record-sets \
  --hosted-zone-id Z03947011ZQ3P0KMABYWB \
  --change-batch file:///tmp/change-dns.json
```

`/tmp/change-dns.json`:
```json
{
  "Changes": [
    {
      "Action": "UPSERT",
      "ResourceRecordSet": {
        "Name": "inboxnegative.com",
        "Type": "A",
        "TTL": 300,
        "ResourceRecords": [
          {
            "Value": "18.237.84.151"
          }
        ]
      }
    }
  ]
}
```

```bash
# WWW subdomain
aws route53 change-resource-record-sets \
  --hosted-zone-id Z03947011ZQ3P0KMABYWB \
  --change-batch file:///tmp/add-www-record.json
```

#### MX Record

For email delivery, an MX record points to the mail subdomain:

`/tmp/mx-update.json`:
```json
{
  "Changes": [
    {
      "Action": "UPSERT",
      "ResourceRecordSet": {
        "Name": "inboxnegative.com",
        "Type": "MX",
        "TTL": 300,
        "ResourceRecords": [
          {
            "Value": "10 mail.inboxnegative.com"
          }
        ]
      }
    },
    {
      "Action": "UPSERT",
      "ResourceRecordSet": {
        "Name": "mail.inboxnegative.com",
        "Type": "A",
        "TTL": 300,
        "ResourceRecords": [
          {
            "Value": "18.237.84.151"
          }
        ]
      }
    }
  ]
}
```

Apply the MX record:

```bash
aws route53 change-resource-record-sets \
  --hosted-zone-id Z03947011ZQ3P0KMABYWB \
  --change-batch file:///tmp/mx-update.json
```

### DNS Propagation

Check DNS propagation:

```bash
# Check A record
dig +short inboxnegative.com
dig +short www.inboxnegative.com

# Check MX record
dig +short MX inboxnegative.com
```

## SSL/HTTPS Setup

### Nginx Installation

Install nginx on the EC2 instance:

```bash
ssh -i inboxnull.pem ec2-user@18.237.84.151

sudo yum update -y
sudo yum install -y nginx
sudo service nginx start
sudo chkconfig nginx on
```

### Initial Nginx Configuration

Create the initial HTTP-only configuration at `/etc/nginx/conf.d/inboxnegative.conf`:

```nginx
server {
    listen 80;
    server_name inboxnegative.com www.inboxnegative.com;

    # ACME challenge directory for SSL verification
    location /.well-known/acme-challenge/ {
        root /var/www/html;
    }

    location / {
        proxy_pass http://localhost:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

Test and reload:

```bash
sudo nginx -t
sudo service nginx reload
```

### SSL Certificate with acme.sh

Install acme.sh:

```bash
curl https://get.acme.sh | sh
source ~/.bashrc
```

Set Let's Encrypt as the default CA:

```bash
~/.acme.sh/acme.sh --set-default-ca --server letsencrypt
```

Create the ACME challenge directory:

```bash
sudo mkdir -p /var/www/html/.well-known/acme-challenge
sudo chown -R ec2-user:ec2-user /var/www/html
```

Obtain the certificate:

```bash
~/.acme.sh/acme.sh --issue \
  -d inboxnegative.com \
  -d www.inboxnegative.com \
  -w /var/www/html
```

Install the certificate to nginx:

```bash
# Create SSL directory
sudo mkdir -p /etc/nginx/ssl
sudo chown ec2-user:ec2-user /etc/nginx/ssl

# Install certificates
~/.acme.sh/acme.sh --install-cert \
  -d inboxnegative.com \
  --cert-file /etc/nginx/ssl/cert.pem \
  --key-file /etc/nginx/ssl/key.pem \
  --fullchain-file /etc/nginx/ssl/fullchain.pem \
  --reloadcmd "sudo service nginx reload"
```

### Final Nginx Configuration with SSL

Update `/etc/nginx/conf.d/inboxnegative.conf`:

```nginx
# HTTP server - redirect to HTTPS
server {
    listen 80;
    server_name inboxnegative.com www.inboxnegative.com;

    # Keep ACME challenge accessible for certificate renewal
    location /.well-known/acme-challenge/ {
        root /var/www/html;
    }

    # Redirect all other HTTP traffic to HTTPS
    location / {
        return 301 https://$host$request_uri;
    }
}

# HTTPS server
server {
    listen 443 ssl http2;
    server_name inboxnegative.com www.inboxnegative.com;

    ssl_certificate /etc/nginx/ssl/fullchain.pem;
    ssl_certificate_key /etc/nginx/ssl/key.pem;

    # Modern SSL configuration
    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_ciphers HIGH:!aNULL:!MD5;
    ssl_prefer_server_ciphers on;

    # Proxy to application
    location / {
        proxy_pass http://localhost:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # WebSocket support for SSE
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_buffering off;
        proxy_cache off;
    }
}
```

Test and reload:

```bash
sudo nginx -t
sudo service nginx reload
```

### Certificate Auto-Renewal

acme.sh automatically sets up a cron job for certificate renewal. Verify:

```bash
crontab -l | grep acme
```

Test renewal (dry run):

```bash
~/.acme.sh/acme.sh --renew \
  -d inboxnegative.com \
  -d www.inboxnegative.com \
  --force
```

## Troubleshooting

### Common Issues

#### 1. Database Connection Failure

**Symptom**: Logs show "Failed to connect to database: DATABASE_URL must be set"

**Solution**: Ensure environment variables are set in the task definition:

```bash
aws ecs describe-task-definition \
  --task-definition inboxnegative-task \
  --query 'taskDefinition.containerDefinitions[0].environment'
```

If missing, update the task definition and redeploy.

#### 2. ECS Agent Not Connected

**Symptom**: Container instance shows as disconnected in ECS console

**Solutions**:

a. Check IAM instance profile:
```bash
aws ec2 describe-instances \
  --instance-ids i-044518e959d9a7a7b \
  --query 'Reservations[0].Instances[0].IamInstanceProfile'
```

b. Reassociate if missing (see IAM Role Configuration above)

c. Check ECS agent status:
```bash
ssh -i inboxnull.pem ec2-user@18.237.84.151
sudo status ecs
```

d. Verify cluster configuration:
```bash
cat /etc/ecs/ecs.config
```

#### 3. Port Binding Conflicts During Deployment

**Symptom**: Service update fails with "no container instance met all of its requirements" or "already using a port required by your task"

**Root Cause**: When `minimumHealthyPercent` is set to 100, ECS tries to start the new task before stopping the old one. On a single EC2 instance, both tasks cannot bind to the same ports (2525, 8080) simultaneously.

**Solution**: Configure the service with `minimumHealthyPercent=0` to allow old task to stop first:

```bash
# Update service deployment configuration
aws ecs update-service \
  --cluster inboxnegative-cluster \
  --service-name inboxnegative-service \
  --deployment-configuration "minimumHealthyPercent=0,maximumPercent=200"

# Force new deployment to apply changes
aws ecs update-service \
  --cluster inboxnegative-cluster \
  --service-name inboxnegative-service \
  --force-new-deployment
```

**Note**: This configuration causes brief downtime during deployments (typically 10-30 seconds) but eliminates the need for manual intervention.

**Alternative (Manual Workaround)**: If you prefer zero downtime and don't mind manual steps:

```bash
# List running tasks
aws ecs list-tasks --cluster inboxnegative-cluster

# Stop old task manually
aws ecs stop-task \
  --cluster inboxnegative-cluster \
  --task <task-arn> \
  --reason "Manual stop for deployment"

# Wait for new task to start
aws ecs describe-services \
  --cluster inboxnegative-cluster \
  --services inboxnegative-service
```

#### 4. Insufficient Memory for Deployment

**Symptom**: Service update fails with "insufficient memory available"

**Solution**:
- Ensure task memory requirements fit within instance capacity
- On t2.micro (1GB RAM), multiple tasks may not fit simultaneously
- Consider using the `minimumHealthyPercent=0` configuration to avoid running multiple tasks

```bash
# List running tasks to check memory usage
aws ecs list-tasks --cluster inboxnegative-cluster

# If needed, stop old task to free memory
aws ecs stop-task \
  --cluster inboxnegative-cluster \
  --task <task-arn> \
  --reason "Freeing memory for new deployment"
```

#### 5. SSL Certificate Verification Fails

**Symptom**: acme.sh shows "Connection refused" during verification

**Causes**:
- Nginx not running on port 80
- Firewall blocking port 80
- DNS not propagated

**Solutions**:

a. Verify nginx is running and listening:
```bash
sudo service nginx status
sudo netstat -tlnp | grep :80
```

b. Test ACME challenge endpoint:
```bash
# Create test file
echo "test" | sudo tee /var/www/html/.well-known/acme-challenge/test.txt

# Test locally
curl http://localhost/.well-known/acme-challenge/test.txt

# Test externally
curl http://inboxnegative.com/.well-known/acme-challenge/test.txt
```

c. Check DNS:
```bash
dig +short inboxnegative.com
```

d. Restart nginx if needed:
```bash
sudo rm -f /var/run/nginx.pid
sudo service nginx start
```

#### 6. Old Cluster Name in ECS Agent

**Symptom**: Agent tries to connect to old cluster (e.g., "inboxnull-cluster")

**Solution**: Delete agent database and restart:

```bash
ssh -i inboxnull.pem ec2-user@18.237.84.151
sudo stop ecs
sudo rm -f /var/lib/ecs/data/agent.db
sudo start ecs
```

## Monitoring and Logs

### CloudWatch Logs

View real-time logs:

```bash
# Tail recent logs
aws logs tail /ecs/inboxnegative --follow

# Filter for specific patterns
aws logs tail /ecs/inboxnegative \
  --since 1h \
  --format short \
  --filter-pattern "ERROR"

# Check database connection
aws logs tail /ecs/inboxnegative \
  --since 5m \
  --format short | grep -i database
```

### ECS Service Health

Check service status:

```bash
aws ecs describe-services \
  --cluster inboxnegative-cluster \
  --services inboxnegative-service \
  --query 'services[0].{Status:status,Running:runningCount,Desired:desiredCount}'
```

Check task status:

```bash
# List tasks
aws ecs list-tasks --cluster inboxnegative-cluster

# Describe specific task
aws ecs describe-tasks \
  --cluster inboxnegative-cluster \
  --tasks <task-arn>
```

### Application Health

Check HTTP endpoint:

```bash
# Test HTTP (should redirect)
curl -I http://inboxnegative.com

# Test HTTPS
curl -I https://inboxnegative.com

# Test with verbose SSL info
curl -vI https://inboxnegative.com 2>&1 | grep -E "SSL|TLS|expire"
```

Check SMTP server:

```bash
telnet inboxnegative.com 2525
# Should see SMTP greeting
```

### System Resources

Monitor EC2 instance resources:

```bash
ssh -i inboxnull.pem ec2-user@18.237.84.151

# Memory usage
free -m

# Docker containers
docker ps

# ECS agent status
sudo status ecs

# Nginx status
sudo service nginx status
```

## Deployment Checklist

When deploying updates:

- [ ] Build and push Docker image to ECR
- [ ] Register new task definition
- [ ] Update ECS service
- [ ] Monitor deployment in ECS console
- [ ] Check CloudWatch logs for errors
- [ ] Verify database connection in logs
- [ ] Test application via HTTPS
- [ ] Verify SSL certificate is valid

## Security Considerations

1. **Environment Variables**: Secrets are stored in ECS task definition (not in code or Docker image)
2. **IAM Roles**: Principle of least privilege applied to instance profile
3. **SSL/TLS**: Modern protocols only (TLSv1.2, TLSv1.3)
4. **Database**: Uses SSL for PostgreSQL connection
5. **SSH Access**: Limited to specific key pair

## Cost Optimization

Current monthly costs (approximate):

- EC2 t2.micro: ~$10/month
- Route53 hosted zone: $0.50/month
- ECR storage: <$1/month (minimal images)
- CloudWatch Logs: <$1/month (minimal logging)
- Data transfer: Variable

**Total**: ~$12-15/month

## Future Improvements

1. **High Availability**: Add Application Load Balancer and multiple AZs
2. **Auto Scaling**: Configure ECS service auto-scaling
3. **Secrets Management**: Use AWS Secrets Manager instead of task definition
4. **CI/CD**: Automate deployments with GitHub Actions or CodePipeline
5. **Monitoring**: Add CloudWatch alarms and dashboards
6. **Backup**: Implement automated EBS snapshots
7. **CDN**: Add CloudFront for static assets

## References

- [AWS ECS Documentation](https://docs.aws.amazon.com/ecs/)
- [acme.sh Documentation](https://github.com/acmesh-official/acme.sh)
- [Nginx Documentation](https://nginx.org/en/docs/)
- [Let's Encrypt](https://letsencrypt.org/)
