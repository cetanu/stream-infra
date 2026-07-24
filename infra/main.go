package main

import (
	_ "embed"
	"fmt"
	"strconv"
	"strings"

	"github.com/dirien/pulumi-vultr/sdk/v2/go/vultr"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi/config"
)

// Embed the startup bash script directly into the compiled Go binary
//
//go:embed scripts/startup.sh
var startupScript string

func main() {
	pulumi.Run(func(ctx *pulumi.Context) error {
		// Load stack configuration
		cfg := config.New(ctx, "")
		region := cfg.Get("region")
		if region == "" {
			region = RegionNewJersey
		}

		// Configure IP whitelist for RTMP (1935/tcp)
		allowedIp := cfg.Get("allowedIngressIp")
		subnetIp := "0.0.0.0"
		subnetSize := 0

		if allowedIp != "" {
			parts := strings.Split(allowedIp, "/")
			subnetIp = parts[0]
			if len(parts) > 1 {
				if sz, err := strconv.Atoi(parts[1]); err == nil {
					subnetSize = sz
				} else {
					subnetSize = 32
				}
			} else {
				subnetSize = 32
			}
		}

		// Optional secret config.toml content passed via Pulumi config
		// Example: pulumi config set rtmpConfig --secret "$(cat config.toml)"
		customConfig := cfg.Get("rtmpConfig")
		userData := startupScript

		if customConfig != "" {
			userData = fmt.Sprintf("export RTMP_CONFIG=%q\n%s", customConfig, startupScript)
		}

		// 1. Private Network / VPC
		vpc, err := vultr.NewVpc(ctx, "stream-vpc", &vultr.VpcArgs{
			Description:  pulumi.String("VPC for RTMP stream-infra services"),
			Region:       pulumi.String(region),
			V4Subnet:     pulumi.String("10.0.0.0"),
			V4SubnetMask: pulumi.Int(24),
		})
		if err != nil {
			return err
		}

		// 2. Firewall Group for RTMP Proxy & Multiplexer
		fwGroup, err := vultr.NewFirewallGroup(ctx, "rtmp-fw-group", &vultr.FirewallGroupArgs{
			Description: pulumi.String("Firewall rules for RTMP Proxy & Multiplexer"),
		})
		if err != nil {
			return err
		}

		// Allow RTMP (1935/tcp) restricted to allowed IP subnet
		_, err = vultr.NewFirewallRule(ctx, "fw-rule-rtmp", &vultr.FirewallRuleArgs{
			FirewallGroupId: fwGroup.ID(),
			Protocol:        pulumi.String("tcp"),
			IpType:          pulumi.String("v4"),
			Subnet:          pulumi.String(subnetIp),
			SubnetSize:      pulumi.Int(subnetSize),
			Port:            pulumi.String("1935"),
			Notes:           pulumi.String("Allow inbound RTMP streaming"),
		})
		if err != nil {
			return err
		}

		// 3. Compute Instance: $6/mo Cloud Compute node
		server, err := vultr.NewInstance(ctx, "rtmp-proxy-node", &vultr.InstanceArgs{
			Plan:            pulumi.String(PlanCloudCompute1vCPU1GB),
			Region:          pulumi.String(region),
			OsId:            pulumi.Int(OsUbuntu2204LTSx64),
			Label:           pulumi.String("rtmp-proxy-node"),
			Hostname:        pulumi.String("rtmp-proxy-1"),
			UserData:        pulumi.String(userData),
			VpcIds:          pulumi.StringArray{vpc.ID()},
			FirewallGroupId: fwGroup.ID(),
			EnableIpv6:      pulumi.Bool(true),
			Backups:         pulumi.String("disabled"),
		})
		if err != nil {
			return err
		}

		// Export outputs
		ctx.Export("vpcId", vpc.ID())
		ctx.Export("firewallGroupId", fwGroup.ID())
		ctx.Export("instanceId", server.ID())
		ctx.Export("instanceMainIp", server.MainIp)

		return nil
	})
}
