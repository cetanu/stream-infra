package main

// Vultr Operating System IDs
const (
	OsUbuntu2204LTSx64 = 1743 // Ubuntu 22.04 LTS x64
	OsUbuntu2404LTSx64 = 2284 // Ubuntu 24.04 LTS x64
	OsDebian12x64      = 2136 // Debian 12 x64
)

// Vultr Instance Plans
const (
	PlanCloudCompute1vCPU1GB = "vc2-1c-1gb" // $6/month Regular Cloud Compute (1 vCPU, 1 GB RAM)
	PlanCloudCompute1vCPU2GB = "vc2-1c-2gb" // 1 vCPU, 2 GB RAM
)

// Common Vultr Regions
const (
	RegionNewJersey = "ewr"
	RegionSydney    = "syd"
	RegionFrankfurt = "fra"
	RegionLondon    = "lhr"
)
