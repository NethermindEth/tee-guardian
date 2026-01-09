# NixOS Host Deployment

This directory contains NixOS configurations for TEE Guardian host machines.

## Available Hosts

| Host | TEE Type | Description |
|------|----------|-------------|
| `open-metal-tdx-dev` | Intel TDX | Open Metal bare metal server with bonding + VLANs |
| `hetzner-sev-dev` | AMD SEV-SNP | Hetzner dedicated server |

## Deployment with nixos-anywhere

### Prerequisites

1. SSH access to the target machine (any Linux distribution)
2. Root access on the target
3. nixos-anywhere available in dev shell: `nix develop`

### Standard Deployment

For hosts with simple networking (single interface, DHCP):

```bash
nixos-anywhere --flake .#<hostname> root@<target-ip>
```

### Complex Networking (Bonding, VLANs)

Hosts with complex networking require a custom kexec installer image that has the network configuration pre-baked. The standard kexec installer cannot restore bonds or VLANs after kexec.

Build the custom kexec installer:

```bash
nix build .#kexec-open-metal-tdx-dev
```

Deploy using the custom kexec image:

```bash
nixos-anywhere \
  --kexec ./result/tarball/nixos-kexec-installer-noninteractive-x86_64-linux.tar.gz \
  --flake .#<hostname> \
  root@<target-ip>
```

Example for Open Metal:

```bash
nix build .#kexec-open-metal-tdx-dev

nixos-anywhere \
  --kexec ./result/tarball/nixos-kexec-installer-noninteractive-x86_64-linux.tar.gz \
  --flake .#open-metal-tdx-dev \
  root@173.231.232.149
```

### Skipping Kexec

If the target already has Nix installed (or you install it manually), you can skip the kexec phase entirely:

```bash
# Install Nix on target first
ssh root@<target-ip> 'sh <(curl -L https://nixos.org/nix/install) --daemon'

# Deploy without kexec
nixos-anywhere --phases disko,install,reboot --flake .#<hostname> root@<target-ip>
```

Note: This approach cannot reformat the drive that the current OS is running from.

## Kexec Installer Configuration

Each host with `kexecInstaller = true` in the flake becomes a kexec installer configuration. The installer:

- Uses the standard NixOS kernel (not the custom TDX/SNP kernel)
- Includes the host's full network configuration (bonds, VLANs, static IPs)
- Sets `system.nixos.variant_id = "installer"` so nixos-anywhere detects it
- Inherits SSH authorized keys from `host.extraAuthorizedKeys`
- Disables disko (disk operations happen in the install phase)

## Adding a New Host

1. Create `nix/hosts/<hostname>.nix` with the host configuration
2. Add network configuration (bonds, VLANs, interfaces, gateway)
3. Add the host to `flake.nix`:

```nix
nixosConfigurations = {
  <hostname> = mkHost "<hostname>" { };
  <hostname>-kexec = mkHost "<hostname>" { kexecInstaller = true; };
};
```

4. If the host has complex networking, add the kexec tarball package:

```nix
packages.${system} = {
  kexec-<hostname> = self.nixosConfigurations.<hostname>-kexec.config.system.build.kexecTarball;
};
```

## Secrets

Secrets are managed with sops-nix. Each host has:

- `nix/secrets/<hostname>.yaml` for host-specific secrets
- `nix/secrets/shared.yaml` for secrets shared across hosts (e.g., Nebula CA)

See `nix/secrets/README.md` for secret management details.
