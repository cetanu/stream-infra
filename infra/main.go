package main

import (
	_ "embed"
	"fmt"
	"net"
	"strconv"
	"strings"

	"github.com/dirien/pulumi-vultr/sdk/v2/go/vultr"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi/config"
)

//go:embed scripts/startup.sh
var startupScript string

func main() {
	pulumi.Run(func(ctx *pulumi.Context) error {
		cfg := config.New(ctx, "")
		region := cfg.Get("region")
		if region == "" {
			region = RegionNewJersey
		}

		allowedIp := cfg.Get("allowedIngressIp")
		if strings.TrimSpace(allowedIp) == "" {
			return fmt.Errorf("'allowedIngressIp' configuration is required. Run: pulumi config set allowedIngressIp <YOUR_IP>")
		}

		subnetIp := strings.TrimSpace(allowedIp)
		subnetSize := 32

		if strings.Contains(subnetIp, "/") {
			parts := strings.Split(subnetIp, "/")
			subnetIp = parts[0]
			if sz, err := strconv.Atoi(parts[1]); err == nil && sz >= 0 && sz <= 32 {
				subnetSize = sz
			} else {
				return fmt.Errorf("Invalid CIDR prefix in 'allowedIngressIp': %q", allowedIp)
			}
		}

		if net.ParseIP(subnetIp) == nil {
			return fmt.Errorf("'allowedIngressIp' %q is not a valid IPv4 address", subnetIp)
		}

		customConfig := cfg.Get("rtmpConfig")
		if strings.TrimSpace(customConfig) == "" {
			return fmt.Errorf("'rtmpConfig' configuration secret is required. Run: pulumi config set rtmpConfig --secret \"$(cat ../config.toml)\"")
		}

		userData := fmt.Sprintf("export RTMP_CONFIG=%q\n%s", customConfig, startupScript)

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

		fwGroup, err := vultr.NewFirewallGroup(ctx, "rtmp-fw-group", &vultr.FirewallGroupArgs{
			Description: pulumi.String("Firewall rules for RTMP Proxy & Multiplexer"),
		})
		if err != nil {
			return err
		}

		_, err = vultr.NewFirewallRule(ctx, "fw-rule-rtmp", &vultr.FirewallRuleArgs{
			FirewallGroupId: fwGroup.ID(),
			Protocol:        pulumi.String("tcp"),
			IpType:          pulumi.String("v4"),
			Subnet:          pulumi.String(subnetIp),
			SubnetSize:      pulumi.Int(subnetSize),
			Port:            pulumi.String("1935"),
			Notes:           pulumi.String("Allow inbound RTMP streaming from whitelisted IP"),
		})
		if err != nil {
			return err
		}

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

		ctx.Export("vpcId", vpc.ID())
		ctx.Export("firewallGroupId", fwGroup.ID())
		ctx.Export("instanceId", server.ID())
		ctx.Export("instanceMainIp", server.MainIp)

		return nil
	})
}
