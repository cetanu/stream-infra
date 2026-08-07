package main

import (
	"encoding/json"
	"fmt"
	"net"
	"strconv"
	"strings"

	"github.com/dirien/pulumi-vultr/sdk/v2/go/vultr"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi/config"
)

type firewallRule struct {
	Name       string `json:"name"`
	Protocol   string `json:"protocol"`
	IPType     string `json:"ipType"`
	Subnet     string `json:"subnet"`
	SubnetSize int    `json:"subnetSize"`
	Port       string `json:"port,omitempty"`
	Notes      string `json:"notes,omitempty"`
}

type infrastructureConfig struct {
	resourcePrefix string
	description    string
	region         string
	plan           string
	osID           int
	label          string
	hostname       string
	vpcSubnet      string
	vpcSubnetMask  int
	enableIPv6     bool
	backups        string
	firewallRules  []firewallRule
}

func main() {
	pulumi.Run(func(ctx *pulumi.Context) error {
		cfg := config.New(ctx, "")

		serversRaw := cfg.Get("servers")
		var serverNamespaces []string
		if serversRaw != "" {
			if err := json.Unmarshal([]byte(serversRaw), &serverNamespaces); err != nil {
				return fmt.Errorf("invalid 'servers' config: %w", err)
			}
		} else {
			serverNamespaces = []string{""}
		}

		for _, ns := range serverNamespaces {
			srvCfg := config.New(ctx, ns)

			stackName := ctx.Stack()
			if ns != "" {
				stackName = stackName + "-" + ns
			}

			infra, err := loadInfrastructureConfig(srvCfg, stackName)
			if err != nil {
				return err
			}

			userData, err := buildHostCloudConfig(cfg, srvCfg)
			if err != nil {
				return err
			}

			vpc, err := vultr.NewVpc(ctx, infra.resourcePrefix+"-vpc", &vultr.VpcArgs{
				Description:  pulumi.String(infra.description),
				Region:       pulumi.String(infra.region),
				V4Subnet:     pulumi.String(infra.vpcSubnet),
				V4SubnetMask: pulumi.Int(infra.vpcSubnetMask),
			})
			if err != nil {
				return err
			}

			fwGroup, err := vultr.NewFirewallGroup(ctx, infra.resourcePrefix+"-fw", &vultr.FirewallGroupArgs{
				Description: pulumi.String(infra.description),
			})
			if err != nil {
				return err
			}

			for _, rule := range infra.firewallRules {
				_, err = vultr.NewFirewallRule(ctx, infra.resourcePrefix+"-allow-"+rule.Name, &vultr.FirewallRuleArgs{
					FirewallGroupId: fwGroup.ID(),
					Protocol:        pulumi.String(rule.Protocol),
					IpType:          pulumi.String(rule.IPType),
					Subnet:          pulumi.String(rule.Subnet),
					SubnetSize:      pulumi.Int(rule.SubnetSize),
					Port:            pulumi.String(rule.Port),
					Notes:           pulumi.String(rule.Notes),
				}, pulumi.IgnoreChanges([]string{"source"}))
				if err != nil {
					return err
				}
			}

			server, err := vultr.NewInstance(ctx, infra.resourcePrefix+"-node", &vultr.InstanceArgs{
				Plan:            pulumi.String(infra.plan),
				Region:          pulumi.String(infra.region),
				OsId:            pulumi.Int(infra.osID),
				Label:           pulumi.String(infra.label),
				Hostname:        pulumi.String(infra.hostname),
				UserData:        userData,
				VpcIds:          pulumi.StringArray{vpc.ID()},
				FirewallGroupId: fwGroup.ID(),
				EnableIpv6:      pulumi.Bool(infra.enableIPv6),
				Backups:         pulumi.String(infra.backups),
			},
				pulumi.DeleteBeforeReplace(true),
				pulumi.IgnoreChanges([]string{"userData"}),
			)
			if err != nil {
				return err
			}

			ctx.Export(infra.resourcePrefix+"-vpcId", vpc.ID())
			ctx.Export(infra.resourcePrefix+"-firewallGroupId", fwGroup.ID())
			ctx.Export(infra.resourcePrefix+"-instanceId", server.ID())
			ctx.Export(infra.resourcePrefix+"-instanceMainIp", server.MainIp)
		}
		return nil
	})
}

func loadInfrastructureConfig(cfg *config.Config, stackName string) (infrastructureConfig, error) {
	prefix := valueOrDefault(cfg.Get("resourcePrefix"), stackName)
	if !validResourceName(prefix) {
		return infrastructureConfig{}, fmt.Errorf("'resourcePrefix' must contain only letters, numbers, and hyphens")
	}

	rules, err := parseFirewallRules(cfg.Require("firewallRules"))
	if err != nil {
		return infrastructureConfig{}, fmt.Errorf("invalid 'firewallRules': %w", err)
	}

	return infrastructureConfig{
		resourcePrefix: prefix,
		description:    valueOrDefault(cfg.Get("description"), "Application host managed by Pulumi"),
		region:         valueOrDefault(cfg.Get("region"), RegionSydney),
		plan:           valueOrDefault(cfg.Get("plan"), PlanCloudCompute1vCPU1GB),
		osID:           intOrDefault(cfg.Get("osId"), OsUbuntu2204LTSx64),
		label:          valueOrDefault(cfg.Get("label"), prefix+"-node"),
		hostname:       valueOrDefault(cfg.Get("hostname"), prefix+"-1"),
		vpcSubnet:      valueOrDefault(cfg.Get("vpcSubnet"), "10.0.0.0"),
		vpcSubnetMask:  intOrDefault(cfg.Get("vpcSubnetMask"), 24),
		enableIPv6:     boolOrDefault(cfg.Get("enableIPv6"), true),
		backups:        valueOrDefault(cfg.Get("backups"), "disabled"),
		firewallRules:  rules,
	}, nil
}

func parseFirewallRules(raw string) ([]firewallRule, error) {
	var rules []firewallRule
	if err := json.Unmarshal([]byte(raw), &rules); err != nil {
		return nil, err
	}
	seen := make(map[string]bool, len(rules))
	for i, rule := range rules {
		if !validResourceName(rule.Name) || seen[rule.Name] {
			return nil, fmt.Errorf("rule %d has an invalid or duplicate name %q", i, rule.Name)
		}
		seen[rule.Name] = true
		if rule.Protocol == "" || (rule.IPType != "v4" && rule.IPType != "v6") {
			return nil, fmt.Errorf("rule %q requires protocol and ipType v4 or v6", rule.Name)
		}
		ip := net.ParseIP(rule.Subnet)
		if ip == nil || (rule.IPType == "v4") != (ip.To4() != nil) {
			return nil, fmt.Errorf("rule %q subnet does not match ipType", rule.Name)
		}
		maxPrefix := 128
		if rule.IPType == "v4" {
			maxPrefix = 32
		}
		if rule.SubnetSize < 0 || rule.SubnetSize > maxPrefix {
			return nil, fmt.Errorf("rule %q has invalid subnetSize", rule.Name)
		}
	}
	return rules, nil
}

func valueOrDefault(value, fallback string) string {
	if strings.TrimSpace(value) == "" {
		return fallback
	}
	return strings.TrimSpace(value)
}

func intOrDefault(value string, fallback int) int {
	parsed, err := strconv.Atoi(value)
	if err != nil {
		return fallback
	}
	return parsed
}

func boolOrDefault(value string, fallback bool) bool {
	parsed, err := strconv.ParseBool(value)
	if err != nil {
		return fallback
	}
	return parsed
}

func validResourceName(value string) bool {
	if value == "" {
		return false
	}
	for _, char := range value {
		if (char < 'a' || char > 'z') && (char < 'A' || char > 'Z') &&
			(char < '0' || char > '9') && char != '-' {
			return false
		}
	}
	return true
}
