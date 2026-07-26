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
	"gopkg.in/yaml.v3"
)

type CloudConfig struct {
	PackageUpdate bool        `yaml:"package_update"`
	Packages      []string    `yaml:"packages"`
	WriteFiles    []WriteFile `yaml:"write_files,omitempty"`
	RunCmd        []string    `yaml:"runcmd"`
}

type WriteFile struct {
	Path        string `yaml:"path"`
	Content     string `yaml:"content"`
	Permissions string `yaml:"permissions,omitempty"`
	Encoding    string `yaml:"encoding,omitempty"`
}

//go:embed scripts/update-ddns.sh
var updateDdnsScript string

//go:embed scripts/update-ddns.service
var updateDdnsService string

//go:embed scripts/setup-rtmp-proxy.sh
var setupRtmpProxyScript string

//go:embed scripts/setup-rtmp-proxy.service
var setupRtmpProxyService string

//go:embed scripts/caddy.service
var caddyService string

func main() {
	pulumi.Run(func(ctx *pulumi.Context) error {
		cfg := config.New(ctx, "")
		region := cfg.Get("region")
		if region == "" {
			region = RegionSydney
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

		customConfig := cfg.RequireSecret("rtmpConfig")
		webUsername := strings.TrimSpace(cfg.Require("webUsername"))
		if webUsername == "" {
			return fmt.Errorf("'webUsername' configuration is required")
		}
		webPassword := cfg.RequireSecret("webPassword")
		deployNonce := strings.TrimSpace(cfg.Get("deployNonce"))
		if deployNonce == "" {
			deployNonce = "initial"
		}

		ddnsHost := cfg.Get("ddnsHost")
		ddnsDomain := cfg.Get("ddnsDomain")
		ddnsPassword := cfg.RequireSecret("ddnsPassword")
		stateVolumeSizeGb := cfg.GetInt("stateVolumeSizeGb")
		if stateVolumeSizeGb == 0 {
			stateVolumeSizeGb = 10
		}
		if stateVolumeSizeGb < 10 {
			return fmt.Errorf("'stateVolumeSizeGb' must be at least 10")
		}

		if ddnsHost == "" {
			ddnsHost = "@"
		}

		fqdn := ddnsDomain
		if ddnsHost != "@" {
			fqdn = fmt.Sprintf("%s.%s", ddnsHost, ddnsDomain)
		}

		userData := pulumi.All(customConfig, webPassword, ddnsPassword).ApplyT(
			func(secrets []interface{}) (string, error) {
				rtmpConfig := secrets[0].(string)
				password := secrets[1].(string)
				ddnsSecret := secrets[2].(string)
				cc := CloudConfig{
					PackageUpdate: true,
					Packages: []string{
						"ffmpeg",
						"curl",
						"ca-certificates",
						"e2fsprogs",
					},
					WriteFiles: []WriteFile{
						{
							Path:        "/run/rtmp-proxy-bootstrap/config.toml",
							Content:     rtmpConfig,
							Permissions: "0600",
						},
						{
							Path: "/etc/rtmp-proxy.env",
							Content: fmt.Sprintf(
								"WEB_AUTH_BOOTSTRAP_USERNAME=%q\nWEB_AUTH_BOOTSTRAP_PASSWORD=%q\n",
								webUsername,
								password,
							),
							Permissions: "0600",
						},
						{
							Path:        "/etc/stream-infra-deploy",
							Content:     deployNonce + "\n",
							Permissions: "0644",
						},
						{
							Path:        "/usr/local/bin/setup-rtmp-proxy.sh",
							Content:     setupRtmpProxyScript,
							Permissions: "0755",
						},
						{
							Path:        "/etc/systemd/system/setup-rtmp-proxy.service",
							Content:     setupRtmpProxyService,
							Permissions: "0644",
						},
						{
							Path:        "/etc/default/update-ddns",
							Content:     fmt.Sprintf("DDNS_HOST=%q\nDDNS_DOMAIN=%q\nDDNS_PASSWORD=%q\n", ddnsHost, ddnsDomain, ddnsSecret),
							Permissions: "0600",
						},
						{
							Path:        "/usr/local/bin/update-ddns.sh",
							Content:     updateDdnsScript,
							Permissions: "0755",
						},
						{
							Path:        "/etc/systemd/system/update-ddns.service",
							Content:     updateDdnsService,
							Permissions: "0644",
						},
						{
							Path:        "/etc/caddy/Caddyfile",
							Content:     fmt.Sprintf("%s {\n\treverse_proxy 127.0.0.1:3000\n}\n", fqdn),
							Permissions: "0644",
						},
						{
							Path:        "/etc/systemd/system/caddy.service",
							Content:     caddyService,
							Permissions: "0644",
						},
					},
					RunCmd: []string{
						"curl -fsSL 'https://caddyserver.com/api/download?os=linux&arch=amd64' -o /usr/local/bin/caddy",
						"chmod +x /usr/local/bin/caddy",
						"systemctl daemon-reload",
						"systemctl enable --now caddy.service",
						"systemctl enable --now setup-rtmp-proxy.service",
						"systemctl enable --now update-ddns.service",
					},
				}

				ccBytes, err := yaml.Marshal(cc)
				if err != nil {
					return "", err
				}
				return "#cloud-config\n" + string(ccBytes), nil
			},
		).(pulumi.StringOutput)

		vpc, err := vultr.NewVpc(ctx, "stream-vpc", &vultr.VpcArgs{
			Description:  pulumi.String("VPC for RTMP stream-infra services"),
			Region:       pulumi.String(region),
			V4Subnet:     pulumi.String("10.0.0.0"),
			V4SubnetMask: pulumi.Int(24),
		})
		if err != nil {
			return err
		}

		fwGroup, err := vultr.NewFirewallGroup(ctx, "stream-fw", &vultr.FirewallGroupArgs{
			Description: pulumi.String("Firewall rules for RTMP Proxy & Multiplexer"),
		})
		if err != nil {
			return err
		}

		_, err = vultr.NewFirewallRule(ctx, "stream-allow-rtmp", &vultr.FirewallRuleArgs{
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

		_, err = vultr.NewFirewallRule(ctx, "stream-allow-http", &vultr.FirewallRuleArgs{
			FirewallGroupId: fwGroup.ID(),
			Protocol:        pulumi.String("tcp"),
			IpType:          pulumi.String("v4"),
			Subnet:          pulumi.String("0.0.0.0"),
			SubnetSize:      pulumi.Int(0),
			Port:            pulumi.String("80"),
			Notes:           pulumi.String("Allow HTTP globally for Let's Encrypt ACME HTTP-01 challenge"),
		})
		if err != nil {
			return err
		}

		_, err = vultr.NewFirewallRule(ctx, "stream-allow-https", &vultr.FirewallRuleArgs{
			FirewallGroupId: fwGroup.ID(),
			Protocol:        pulumi.String("tcp"),
			IpType:          pulumi.String("v4"),
			Subnet:          pulumi.String(subnetIp),
			SubnetSize:      pulumi.Int(subnetSize),
			Port:            pulumi.String("443"),
			Notes:           pulumi.String("Allow HTTPS from whitelisted IP"),
		})
		if err != nil {
			return err
		}

		server, err := vultr.NewInstance(ctx, "stream-rtmp-node", &vultr.InstanceArgs{
			Plan:            pulumi.String(PlanCloudCompute1vCPU1GB),
			Region:          pulumi.String(region),
			OsId:            pulumi.Int(OsUbuntu2204LTSx64),
			Label:           pulumi.String("rtmp-proxy-node"),
			Hostname:        pulumi.String("rtmp-proxy-1"),
			UserData:        userData,
			VpcIds:          pulumi.StringArray{vpc.ID()},
			FirewallGroupId: fwGroup.ID(),
			EnableIpv6:      pulumi.Bool(true),
			Backups:         pulumi.String("disabled"),
		},
			pulumi.DeleteBeforeReplace(true),
			pulumi.ReplaceOnChanges([]string{"userData"}),
		)
		if err != nil {
			return err
		}

		stateVolume, err := vultr.NewBlockStorage(ctx, "stream-state", &vultr.BlockStorageArgs{
			AttachedToInstance: server.ID(),
			BlockType:          pulumi.String("storageOpt"),
			Label:              pulumi.String("rtmp-proxy-state"),
			Live:               pulumi.Bool(true),
			Region:             pulumi.String(region),
			SizeGb:             pulumi.Int(stateVolumeSizeGb),
		}, pulumi.Protect(true), pulumi.RetainOnDelete(true))
		if err != nil {
			return err
		}

		ctx.Export("vpcId", vpc.ID())
		ctx.Export("firewallGroupId", fwGroup.ID())
		ctx.Export("instanceId", server.ID())
		ctx.Export("instanceMainIp", server.MainIp)
		ctx.Export("stateVolumeId", stateVolume.ID())
		ctx.Export("stateVolumeMountId", stateVolume.MountId)

		return nil
	})
}
