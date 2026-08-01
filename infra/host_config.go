package main

import (
	_ "embed"
	"encoding/json"
	"fmt"
	"strings"

	"github.com/pulumi/pulumi/sdk/v3/go/pulumi"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi/config"
	"gopkg.in/yaml.v3"
)

type cloudConfig struct {
	PackageUpdate bool        `yaml:"package_update"`
	Packages      []string    `yaml:"packages"`
	WriteFiles    []writeFile `yaml:"write_files"`
	RunCommands   []string    `yaml:"runcmd"`
}

type writeFile struct {
	Path        string `yaml:"path"`
	Content     string `yaml:"content"`
	Permissions string `yaml:"permissions"`
}

type webhookConfig struct {
	Secret        string  `json:"secret"`
	RepositoryIDs []int64 `json:"repositoryIds"`
	Event         string  `json:"event"`
	Action        string  `json:"action"`
	Path          string  `json:"path"`
	ListenAddress string  `json:"listenAddress"`
	ListenPort    int     `json:"listenPort"`
}

type deploymentConfig struct {
	StateRepositories []stateRepository
	WebhookHost       string
	WebhookPath       string
	Event             string
	Action            string
	ListenPort        int
}

type stateRepository struct {
	URL          string `json:"url"`
	Branch       string `json:"branch"`
	Root         string `json:"root"`
	State        string `json:"state"`
	RepositoryID int64  `json:"repositoryId"`
}

type saltMinionConfig struct {
	FileClient        string                           `yaml:"file_client"`
	FileserverBackend []string                         `yaml:"fileserver_backend"`
	FileRoots         map[string][]string              `yaml:"file_roots"`
	PillarRoots       map[string][]string              `yaml:"pillar_roots"`
	GitFSProvider     string                           `yaml:"gitfs_provider"`
	GitFSRemotes      []map[string][]map[string]string `yaml:"gitfs_remotes"`
}

//go:embed host/deployment-webhook.py
var deploymentWebhook string

//go:embed host/deployment-webhook.service
var deploymentWebhookService string

//go:embed host/deployment-reconcile.path
var deploymentReconcilePath string

//go:embed host/deployment-reconcile.service
var deploymentReconcileService string

//go:embed host/reconcile-deployments.sh
var reconcileDeployments string

//go:embed host/caddy.service
var caddyService string

func buildHostCloudConfig(cfg *config.Config) (pulumi.StringOutput, error) {
	deployment, err := loadDeploymentConfig(cfg)
	if err != nil {
		return pulumi.StringOutput{}, err
	}

	return pulumi.All(cfg.RequireSecret("webhookSecret"), cfg.RequireSecret("pillar")).ApplyT(func(secrets []interface{}) (string, error) {
		secret := secrets[0].(string)
		pillar := secrets[1].(string)
		repositoryIDs := make([]int64, 0, len(deployment.StateRepositories))
		gitFSRemotes := make([]map[string][]map[string]string, 0, len(deployment.StateRepositories))
		states := make([]string, 0, len(deployment.StateRepositories))
		for _, repository := range deployment.StateRepositories {
			repositoryIDs = append(repositoryIDs, repository.RepositoryID)
			gitFSRemotes = append(gitFSRemotes, map[string][]map[string]string{
				repository.URL: {
					{"base": repository.Branch},
					{"root": repository.Root},
				},
			})
			states = append(states, repository.State)
		}

		listenerJSON, err := json.Marshal(webhookConfig{
			Secret:        secret,
			RepositoryIDs: repositoryIDs,
			Event:         deployment.Event,
			Action:        deployment.Action,
			Path:          deployment.WebhookPath,
			ListenAddress: "127.0.0.1",
			ListenPort:    deployment.ListenPort,
		})
		if err != nil {
			return "", err
		}

		caddyConfig := fmt.Sprintf(
			"%s {\n\thandle %s {\n\t\treverse_proxy 127.0.0.1:%d\n\t}\n\timport /etc/caddy/apps/*.caddy\n}\n",
			deployment.WebhookHost,
			deployment.WebhookPath,
			deployment.ListenPort,
		)
		saltConfigBytes, err := yaml.Marshal(saltMinionConfig{
			FileClient:        "local",
			FileserverBackend: []string{"roots", "gitfs"},
			FileRoots:         map[string][]string{"base": {"/etc/salt/roots"}},
			PillarRoots:       map[string][]string{"base": {"/etc/salt/pillar"}},
			GitFSProvider:     "gitpython",
			GitFSRemotes:      gitFSRemotes,
		})
		if err != nil {
			return "", err
		}
		topBytes, err := yaml.Marshal(map[string]map[string][]string{
			"base": {"*": states},
		})
		if err != nil {
			return "", err
		}
		saltConfig := string(saltConfigBytes)
		pillarTop := "base:\n  '*':\n    - application\n"

		cc := cloudConfig{
			PackageUpdate: true,
			Packages: []string{
				"ca-certificates",
				"curl",
				"git",
				"python3",
				"python3-git",
				"salt-minion",
			},
			WriteFiles: []writeFile{
				{Path: "/usr/local/libexec/deployment-webhook", Content: deploymentWebhook, Permissions: "0755"},
				{Path: "/usr/local/libexec/reconcile-deployments", Content: reconcileDeployments, Permissions: "0755"},
				{Path: "/etc/deployment-webhook.json", Content: string(listenerJSON) + "\n", Permissions: "0600"},
				{Path: "/etc/salt/minion.d/deployment.conf", Content: saltConfig, Permissions: "0644"},
				{Path: "/etc/salt/roots/top.sls", Content: string(topBytes), Permissions: "0644"},
				{Path: "/etc/salt/pillar/top.sls", Content: pillarTop, Permissions: "0644"},
				{Path: "/etc/salt/pillar/application.sls", Content: pillar, Permissions: "0600"},
				{Path: "/etc/caddy/Caddyfile", Content: caddyConfig, Permissions: "0644"},
				{Path: "/etc/caddy/apps/empty.caddy", Content: "# Application routes are managed by Salt.\n", Permissions: "0644"},
				{Path: "/etc/systemd/system/deployment-webhook.service", Content: deploymentWebhookService, Permissions: "0644"},
				{Path: "/etc/systemd/system/deployment-reconcile.path", Content: deploymentReconcilePath, Permissions: "0644"},
				{Path: "/etc/systemd/system/deployment-reconcile.service", Content: deploymentReconcileService, Permissions: "0644"},
				{Path: "/etc/systemd/system/caddy.service", Content: caddyService, Permissions: "0644"},
			},
			RunCommands: []string{
				"id deployment-webhook >/dev/null 2>&1 || useradd --system --home-dir /nonexistent --shell /usr/sbin/nologin deployment-webhook",
				"id caddy >/dev/null 2>&1 || useradd --system --home-dir /var/lib/caddy --create-home --shell /usr/sbin/nologin caddy",
				"chown deployment-webhook:deployment-webhook /etc/deployment-webhook.json",
				"install -d -o deployment-webhook -g deployment-webhook -m 0750 /var/lib/deployment-reconciler",
				"install -d -o root -g root -m 0755 /etc/caddy/apps",
				"curl -fsSL --retry 3 'https://caddyserver.com/api/download?os=linux&arch=amd64' -o /usr/local/bin/caddy",
				"chmod 0755 /usr/local/bin/caddy",
				"systemctl disable --now salt-minion.service || true",
				"systemctl daemon-reload",
				"systemctl enable --now caddy.service deployment-webhook.service deployment-reconcile.path",
				"install -o deployment-webhook -g deployment-webhook -m 0640 /dev/null /var/lib/deployment-reconciler/pending",
			},
		}

		contents, err := yaml.Marshal(cc)
		if err != nil {
			return "", err
		}
		return "#cloud-config\n" + string(contents), nil
	}).(pulumi.StringOutput), nil
}

func loadDeploymentConfig(cfg *config.Config) (deploymentConfig, error) {
	var repositories []stateRepository
	if err := json.Unmarshal([]byte(cfg.Require("stateRepositories")), &repositories); err != nil {
		return deploymentConfig{}, fmt.Errorf("invalid 'stateRepositories': %w", err)
	}
	if len(repositories) == 0 {
		return deploymentConfig{}, fmt.Errorf("'stateRepositories' must contain at least one repository")
	}
	seenStates := make(map[string]bool, len(repositories))
	seenRepositoryIDs := make(map[int64]bool, len(repositories))
	for index := range repositories {
		repository := &repositories[index]
		repository.URL = strings.TrimSpace(repository.URL)
		repository.Branch = valueOrDefault(repository.Branch, "master")
		repository.Root = valueOrDefault(repository.Root, "salt")
		repository.State = strings.TrimSpace(repository.State)
		if repository.URL == "" || !validGitRef(repository.Branch) ||
			strings.HasPrefix(repository.Root, "/") || strings.Contains(repository.Root, "..") || !validGitRef(repository.Root) ||
			!validGitRef(repository.State) || repository.RepositoryID <= 0 ||
			seenStates[repository.State] || seenRepositoryIDs[repository.RepositoryID] {
			return deploymentConfig{}, fmt.Errorf("state repository %d is invalid or duplicates a state/repository ID", index)
		}
		seenStates[repository.State] = true
		seenRepositoryIDs[repository.RepositoryID] = true
	}
	host := strings.TrimSpace(cfg.Require("webhookHost"))
	if !validHostname(host) {
		return deploymentConfig{}, fmt.Errorf("'webhookHost' must be a DNS hostname")
	}
	path := valueOrDefault(cfg.Get("webhookPath"), "/hooks/github")
	if !validWebhookPath(path) {
		return deploymentConfig{}, fmt.Errorf("'webhookPath' must be an absolute URL path")
	}
	port := intOrDefault(cfg.Get("webhookListenPort"), 9100)
	if port < 1024 || port > 65535 {
		return deploymentConfig{}, fmt.Errorf("'webhookListenPort' must be between 1024 and 65535")
	}
	return deploymentConfig{
		StateRepositories: repositories,
		WebhookHost:       host,
		WebhookPath:       path,
		Event:             valueOrDefault(cfg.Get("webhookEvent"), "release"),
		Action:            valueOrDefault(cfg.Get("webhookAction"), "published"),
		ListenPort:        port,
	}, nil
}

func validGitRef(value string) bool {
	if value == "" || strings.HasPrefix(value, "-") || strings.Contains(value, "..") {
		return false
	}
	for _, char := range value {
		if (char < 'a' || char > 'z') && (char < 'A' || char > 'Z') &&
			(char < '0' || char > '9') && !strings.ContainsRune("-._/", char) {
			return false
		}
	}
	return true
}

func validHostname(value string) bool {
	if value == "" || strings.HasPrefix(value, ".") || strings.HasSuffix(value, ".") {
		return false
	}
	for _, char := range value {
		if (char < 'a' || char > 'z') && (char < 'A' || char > 'Z') &&
			(char < '0' || char > '9') && char != '-' && char != '.' {
			return false
		}
	}
	return true
}

func validWebhookPath(value string) bool {
	if !strings.HasPrefix(value, "/") {
		return false
	}
	for _, char := range value {
		if (char < 'a' || char > 'z') && (char < 'A' || char > 'Z') &&
			(char < '0' || char > '9') && !strings.ContainsRune("/-._~", char) {
			return false
		}
	}
	return true
}
