## TDX Guest QEMU Commands


```bash

# TDX Guest Launch Command
qemu-system-x86_64 -machine q35,accel=kvm,kernel-irqchip=split,confidential-guest-support=tdx,hpet=off -cpu host -smp 8 -m 16G -object '{"qom-type":"tdx-guest","id":"tdx","quote-generation-socket":{"type":"vsock","cid":"2","port":"4050"}}' -bios result/OVMF_DEBUG.fd -drive file=boot.img,format=raw,if=virtio -nographic

# TDX Guest with Tools Image
qemu-system-x86_64 -machine q35,accel=kvm,kernel-irqchip=split,confidential-guest-support=tdx,hpet=off -cpu host -smp 8 -m 16G -object '{"qom-type":"tdx-guest","id":"tdx","quote-generation-socket":{"type":"vsock","cid":"2","port":"4050"}}' -bios result/OVMF_DEBUG.fd -drive file=boot.img,format=raw,if=virtio -drive file=tools.img,format=raw,if=virtio -nographic

# TDX Guest w/ PCI passthrough
qemu-system-x86_64 -machine q35,accel=kvm,kernel-irqchip=split,confidential-guest-support=tdx,hpet=off -cpu host -smp 8 -m 16G -object '{"qom-type":"tdx-guest","id":"tdx","quote-generation-socket":{"type":"vsock","cid":"2","port":"4050"}}' -bios result/OVMF_DEBUG.fd -drive file=boot.img,format=raw,if=virtio -object iommufd,id=iommufd0 -fw_cfg name=opt/ovmf/X-PciMmio64Mb1,string=65536 -device pcie-root-port,port=16,chassis=1,id=pci.1,bus=pcie.0 -device vfio-pci,host=3d:00.0,bus=pci.1,iommufd=iommufd0 -nographic

```
## Stress-ng Debian Equivalent Command

stress-ng --cpu 4 --io 2 --vm 2 --vm-bytes 1G --vm-method cache-lines --vm-method cache-stripe --vm-method checkerboard --vm-method flip --vm-method fwdrev --vm-method galpat-0 --vm-method galpat-1 --vm-method gray --vm-method grayflip --vm-method rowhammer --vm-method incdec --vm-method inc-nybble --vm-method lfsr32 --vm-method rand-set --vm-method rand-sum --vm-method read64 --vm-method ror --vm-method swap --vm-method move-inv --vm-method modulo-x --vm-method mscan --vm-method wrrd128nt --vm-method prime-0 --vm-method prime-1 --vm-method prime-gray-0 --vm-method prime-gray-1 --vm-method prime-incdec --vm-method walk-0d --vm-method walk-1d --vm-method walk-0a --vm-method walk-1a --vm-method write64 --vm-method write64nt --vm-method write1024v --vm-method zero-one --timeout 30s --metrics-brief

## TSM Report Verification

```bash

cd /sys/kernel/config/tsm/report

mkdir test_report
dd if=/dev/zero of=inblob bs=64 count=1
echo 1 > generation

# Wait until generation returns 1
cat generation

hexdump -C outblob

```

```bash

#!/usr/bin/env bash
set -euo pipefail

REPORT="benchmark_report.txt"
: > "$REPORT"

echo "=== CPU TEST ===" | tee -a "$REPORT"
sysbench cpu --cpu-max-prime=50000 --time=60 --threads=1 run | tee -a "$REPORT"
openssl speed --seconds 30 aes-256-cbc rsa2048 sha256 | tee -a "$REPORT"

echo "=== MEMORY TEST ===" | tee -a "$REPORT"
sysbench memory --memory-total-size=16G --memory-block-size=1Kb \
  --memory-oper=write --memory-access-mode=rnd --threads=1 run | tee -a "$REPORT"

echo "=== DISK TESTS ===" | tee -a "$REPORT"

echo "--- Throughput (iodepth=1) ---" | tee -a "$REPORT"
fio --name=writefile --runtime=30 --filename=/mnt/testfile --size=50G --bs=1M --nrfiles=1 --direct=1 --sync=0 --randrepeat=0 --rw=write --end_fsync=1 --iodepth=1 --ioengine=libaio | tee -a "$REPORT"

echo "--- Throughput (iodepth=128) ---" | tee -a "$REPORT"
fio --name=writefile --runtime=30 --filename=/mnt/testfile --size=50G --bs=1M --nrfiles=1 --direct=1 --sync=0 --randrepeat=0 --rw=write --end_fsync=1 --iodepth=128 --ioengine=libaio | tee -a "$REPORT"

echo "--- Latency (iodepth=1) ---" | tee -a "$REPORT"
fio --time_based --name=benchmark --runtime=30 --filename=/mnt/testfile --ioengine=libaio --randrepeat=0 --iodepth=1 --direct=1 --invalidate=1 --verify=0 --verify_fatal=0 --numjobs=1 --rw=randwrite --blocksize=4k --group_reporting --norandommap | tee -a "$REPORT"

echo "--- Latency (iodepth=128) ---" | tee -a "$REPORT"
fio --time_based --name=benchmark --runtime=30 --filename=/mnt/testfile --ioengine=libaio --randrepeat=0 --iodepth=128 --direct=1 --invalidate=1 --verify=0 --verify_fatal=0 --numjobs=1 --rw=randwrite --blocksize=4k --group_reporting --norandommap | tee -a "$REPORT"

echo "=== NETWORK TEST -- 1x10 host in London ===" | tee -a "$REPORT"
iperf3 -c lon.speedtest.clouvider.net -p 5200-5208 -t 30 | tee -a "$REPORT"

echo "=== NETWORK TEST -- 1x10 host in Dallas ===" | tee -a "$REPORT"
iperf3 -c dal.speedtest.clouvider.net -p 5200-5209 -t 30 | tee -a "$REPORT"

echo "=== NETWORK TEST -- 1x10 host in Frankfurt ===" | tee -a "$REPORT"
iperf3 -c fra.speedtest.clouvider.net -p 5200-5209 -t 30 | tee -a "$REPORT"

echo "=== STRESS TEST (CPU, memory, I/O) ===" | tee -a "$REPORT"
# This is the debian-equivalent command for stress-ng
stress-ng --cpu 4 --io 2 --vm 2 --vm-bytes 1G --vm-method cache-lines --vm-method cache-stripe --vm-method checkerboard --vm-method flip --vm-method fwdrev --vm-method galpat-0 --vm-method galpat-1 --vm-method gray --vm-method grayflip --vm-method rowhammer --vm-method incdec --vm-method inc-nybble --vm-method lfsr32 --vm-method rand-set --vm-method rand-sum --vm-method read64 --vm-method ror --vm-method swap --vm-method move-inv --vm-method modulo-x --vm-method mscan --vm-method wrrd128nt --vm-method prime-0 --vm-method prime-1 --vm-method prime-gray-0 --vm-method prime-gray-1 --vm-method prime-incdec --vm-method walk-0d --vm-method walk-1d --vm-method walk-0a --vm-method walk-1a --vm-method write64 --vm-method write64nt --vm-method write1024v --vm-method zero-one --timeout 30s --metrics-brief | tee -a "$REPORT"

echo "Report saved to $REPORT"

```
