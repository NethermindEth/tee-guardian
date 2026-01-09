# Secrets Management with sops-nix

This directory contains encrypted secrets managed by [sops-nix](https://github.com/Mic92/sops-nix).

### Server Setup

```bash

sudo mkdir -p /var/lib/sops-nix
sudo age-keygen -o /var/lib/sops-nix/key.txt
# Example output:
# public key: age1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx

```
### Laptop/Devbox Setup

```bash

# Install age and sops
mkdir ~/.config/sops/age/
age-keygen -o ~/.config/sops/age/keys.txt
# Example output:
# public key: age1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx

```

### Update .sops.yaml with Public Key

Edit `nixos-tdx/secrets/.sops.yaml`

```yaml
keys:
  - &server-hostname age1xxxxxSERVER-PUBKEYxxxx...
  - &laptop-hostname age1xxxxxLAPTOP-PUBKEYxxxx...
```

### Re-encrypt secrets with new public keys
```bash
# After updating the .sops.yaml file, you can re-encrypt secrets files with the new public keys
cd nixos-tdx/secrets
sops updatekeys ovh-tdx-development.yaml
# Example output:
# 2025/12/02 13:45:37 Syncing keys for file .../nixos-tdx/secrets/ovh-tdx-development.yaml
# The following changes will be made to the file's groups:
# Group 1
#     age1fd7u2ec88u5v74q3qj8dxrsaw2f4md9lnt0klnr9vf74exr603mqw3qhey
# +++ age13g6ppehv0chlfnkgl7p0cd6lcas3wq5phfv9wzwvhfr0xhjnvcfsnxktlg
# Is this okay? (y/n):y
# 2025/12/02 13:45:41 File .../nixos-tdx/secrets/ovh-tdx-development.yaml synced with new keys

```

The .sops.yaml file includes a list of paths that should be encrypted.  You set the public keys of all keys that should be able to decrypt the secrets for that path.

For the ovh-tdx-development.yaml secrets file, the public key for that OVH host should be added to the list, as well as the developer keys that need to manage that server.  This makes it easy to
create these secrets, and specify which users/machines need access

```bash

# Create encrypted secrets file
cd nixos-tdx/secrets
sops ovh-tdx-development.yaml

# This will open your editor. Add the following secrets:
#
# intel-pcs-api-key: "your-48-char-intel-pcs-api-key-here"
# intel-trust-authority-api-key: "your-intel-trust-authority-api-key-here"
#
# Note:
# - intel-pcs-api-key: Used by host PCCS for fetching platform certificates
# - intel-trust-authority-api-key: Used by guest VM for remote attestation verification

# Save and close the file. The file will be encrypted automatically.

# Verify the encrypted file has no plaintext
cat ovh-tdx-development.yaml
# Example output:
# intel-pcs-api-key: ENC[AES256_GCM,data:encrypted-data-here,type:str]
# intel-trust-authority-api-key: ENC[AES256_GCM,data:encrypted-data-here,type:str]
# sops:
#     age:
#         - recipient: age1xxxxxxxxx...
#           enc: encrypted-key-here

```

