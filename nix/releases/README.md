# Nix Build System

This directory contains versioned, immutable build configurations for the TEE Key Management System.

## Immutability Principle

**CRITICAL**: Once a version file (e.g., `v0.1.0-tdx.nix`) is released and tagged, any changes to anything will be reflected in a new version. This ensures:

- **Reproducible Builds**: Anyone can rebuild identical binaries from the same version file
- **Security Verification**: RTMR measurements remain consistent across builds
- **Audit Trail**: Complete history of all build configurations is preserved
- **Trust**: Users can verify that published binaries match the source

## Build Outputs

Each version file produces:

- **bzImage**: compressed linux kernel with TDX-Guest support & required drivers
- **OVMF.fd**: TDX-Enabled firmware for booting the kernel
- **rootfs.img**: Root filesystem with userspace tools/applications, and the TEE Key Management System Binaries that get executed

## Installing Nix

```bash

# Ensure Nix is installed
curl -L https://nixos.org/nix/install | sh

# Add the following to your PATH
# /nix/var/nix/profiles/default/bin
# $HOME/.nix-profile/bin

nix --version
```

## Build Instructions

```bash

# Install rust-overlay for Nix (for pinned Rust versions)
nix-channel --add https://github.com/oxalica/rust-overlay/archive/master.tar.gz rust-overlay
nix-channel --update

nix build nix/v0.1.0-tdx.nix
```

## Version History

| Version | Git Tag | Kernel | Rust   | Status | Notes                  |
|---------|---------|--------|--------|--------|------------------------|
| 0.1.0   | v0.1.0  | 6.15.8 | 1.67.1 | Draft  |                        |

## Security Considerations

### Source Verification

Each build fetches source code from a specific git tag with cryptographic verification:

```nix
src = pkgs.fetchFromGitHub {
  owner = "nethermindEth";
  repo = "tee-kms";
  rev = "v0.1.0";                                        # Exact git tag
  sha256 = "sha256-AAAA...";                             # Cryptographic hash
};
```

### Dependency Pinning

All dependencies are pinned to exact versions:

- **Kernel**: Specific Linux version with known hash
- **Rust**: Exact toolchain version from rust-overlay
- **Libraries**: All Nix packages with SHA256 verification

### TPM Measurements

The kernel configuration directly affects TPM PCR values:

- **PCR 8**: Kernel measurement (affected by config changes)
- **PCR 9**: Kernel command line measurement
- **PCR 10**: Initrd measurement
- **PCR 11**: Application binary measurement

Any change to the kernel config will change PCR 8, requiring updates to attestation verification.

## Troubleshooting

### Build Failures

```bash
# Clean build cache
nix-collect-garbage

# Build with verbose output
nix build nix/v0.1.0-snp.nix --verbose

```

### Hash Mismatches

```bash
# Update source hash
nix-prefetch-github nethermindEth tee-kms --rev v0.1.0

# Update kernel hash
nix-prefetch-url mirror://kernel/linux/kernel/v6.x/linux-6.6.69.tar.xz
```

### Reproducibility Issues

```bash
# Compare builds
nix build nix/v0.1.0-snp.nix
nix build nix/v0.1.0-snp.nix --keep-failed --rebuild

# Check for non-deterministic elements
diffoscope /nix/store/...-linux-6.15.4/ /nix/store/...-linux-6.15.4.check/
```



## Development Tools in VM

The test VM can include development tools (cargo, rustc, rust-analyzer) via Nix bundles. These are self-contained executables that work without `/nix/store`.

### Setup (One-time)

```bash
# Create vm-tools directory
mkdir vm-tools

# Bundle development tools
nix bundle nixpkgs#cargo -o vm-tools/cargo
nix bundle nixpkgs#rustc -o vm-tools/rustc
nix bundle nixpkgs#gcc -o vm-tools/gcc
nix bundle nixpkgs#stress-ng -o vm-tools/stress-ng
```

### Build VM with Tools

```bash
# Build test VM (will include tools from vm-tools/ if present)
nix build -f nix/test-vm.nix --print-build-logs

```

### Inside the VM

Tools are automatically mounted at `/tools/bin`:

```bash
# SSH into VM
guardian ssh

# Use tools
cargo --version
```


