use anyhow::Result;
use std::{
    ops::Add,
    process::{Command, Stdio},
};
use structopt;

// QEMU_OPTS_NET1="192.168.1.0/24"
// QEMU_OPTS_NET1_FIRST_IP="192.168.1.10"
// QEMU_OPTS_NET2="192.168.2.0/24"
// QEMU_OPTS_NET2_FIRST_IP="192.168.2.10"

// QEMU_MEMORY=4096

// DIST=$PWD/dist/amd64

// SSH_PORT=2222
// QEMU_TFTP_OPTS=

// QEMU_OPTS_BIOS="-drive if=pflash,format=raw,unit=0,readonly=on,file=$DIST/OVMF_CODE.fd -drive if=pflash,format=raw,unit=1,file=$DIST/OVMF_VARS.fd"
// #QEMU_OPTS_BIOS_=-bios "$DIST/OVMF.fd"
// #QEMU_OPTS_BIOS=$(QEMU_OPTS_BIOS_$(PFLASH))

// QEMU_OPTS_COMMON=-smbios type=1,serial=13471118 -m "$QEMU_MEMORY" -smp 4 -display sdl "$QEMU_OPTS_BIOS" \
// -serial mon:stdio \
// -rtc base=utc,clock=rt \
// -netdev user,id=eth0,net=$QEMU_OPTS_NET1,dhcpstart=$QEMU_OPTS_NET1_FIRST_IP,hostfwd=tcp::$SSH_PORT-:22$QEMU_TFTP_OPTS -device virtio-net-pci,netdev=eth0,romfile="" \
// -netdev user,id=eth1,net=$QEMU_OPTS_NET2,dhcpstart=$QEMU_OPTS_NET2_FIRST_IP -device virtio-net-pci,netdev=eth1,romfile=""

#[derive(Debug)]
struct NetDevice {
    id: String,
    mask: Option<String>,
    dhcp_start: Option<String>,
    hostwfd: Vec<(u32, u32)>,
}

impl NetDevice {
    fn new<T: Into<String>>(id: T) -> Self {
        Self {
            id: id.into(),
            mask: None,
            dhcp_start: None,
            hostwfd: Vec::new(),
        }
    }

    fn mask<T: Into<String>>(mut self, mask: T) -> Self {
        self.mask = Some(mask.into());
        self
    }

    fn dhcp_start<T: Into<String>>(mut self, ip: T) -> Self {
        self.dhcp_start = Some(ip.into());
        self
    }

    fn port_forward(mut self, host_port: u32, vm_port: u32) -> Self {
        self.hostwfd.push((host_port, vm_port));
        self
    }

    fn to_string(&self) -> Vec<String> {
        let mut result = Vec::new();
        result.push("-netdev".to_string());
        let mut param = String::new();
        param.push_str(format!("user,id={}", self.id).as_str());
        if let Some(mask) = &self.mask {
            param.push_str(format!(",net={}", mask).as_str());
        }
        if let Some(ip) = &self.dhcp_start {
            param.push_str(format!(",dhcpstart={}", ip).as_str());
        }

        self.hostwfd
            .iter()
            .for_each(|e| param.push_str(format!(",hostfwd=tcp::{}-:{}", e.0, e.1).as_str()));

        result.push(param);

        result.push("-device".to_string());
        result.push(format!("virtio-net-pci,netdev={},romfile=", self.id));

        result
    }
}

#[derive(Debug)]
struct QemuInstance {
    cmd: Vec<String>,
    nets: Vec<NetDevice>,
    base_path: String,
}

impl QemuInstance {
    fn new<T: Into<String>>(base_path: T) -> Self {
        Self {
            cmd: Vec::new(),
            nets: Vec::new(),
            base_path: base_path.into(),
        }
    }
    fn machine(&mut self) -> &mut Self {
        self.cmd.push("-machine".to_string());
        self.cmd
            .push("q35,accel=kvm,usb=off,dump-guest-core=off,kernel-irqchip=split".to_string());
        self
    }
    fn cpu(&mut self) -> &mut Self {
        self.cmd.push("-cpu".to_string());
        self.cmd.push("host".to_string());
        self
    }
    fn iommu(&mut self) -> &mut Self {
        self.cmd.push("-device".to_string());
        self.cmd
            .push("intel-iommu,intremap=on,caching-mode=on,aw-bits=48".to_string());
        self
    }
    fn serial(&mut self) -> &mut Self {
        self.cmd.push("-serial".to_string());
        self.cmd.push("mon:stdio".to_string());
        self
    }
    fn video<T: Into<String>>(&mut self, t: T) -> &mut Self {
        self.cmd.push("-display".to_string());
        self.cmd.push(t.into());
        self
    }
    fn eve_serial<T: Into<String>>(&mut self, serial: T) -> &mut Self {
        self.cmd.push("-smbios".to_string());
        self.cmd.push(format!("type=1,serial={}", serial.into()));
        self
    }
    fn ram(&mut self, ram: u32) -> &mut Self {
        self.cmd.push("-m".to_string());
        self.cmd.push(ram.to_string());
        self
    }
    fn rtc(&mut self) -> &mut Self {
        self.cmd.push("-rtc".to_string());
        self.cmd.push("base=utc,clock=rt".to_string());
        self
    }

    fn bios_file<T: Into<String>>(&mut self, file: T, segment: u32) -> &mut Self {
        self.cmd.push("-drive".to_string());
        self.cmd.push(format!(
            "if=pflash,format=raw,unit={},readonly=on,file={}/installer/firmware/{}",
            segment,
            self.base_path,
            file.into()
        ));
        self
    }
    fn drive<T: Into<String>>(&mut self, image: T) -> &mut Self {
        self.cmd.push("-drive".to_string());
        self.cmd.push(format!(
            "file={}/{},format=qcow2,id=uefi-disk",
            self.base_path,
            image.into()
        ));
        self
    }

    fn net(&mut self, net: NetDevice) -> &mut Self {
        self.nets.push(net);
        self
    }

    fn tpm(&mut self) -> &mut Self {
        self.cmd.push("-tpmdev".to_string());
        self.cmd.push("emulator,id=tpm0,chardev=chrtpm".to_string());
        self.cmd.push("-device".to_string());
        self.cmd.push("tpm-tis,tpmdev=tpm0".to_string());
        self.cmd.push("-chardev".to_string());
        self.cmd
            .push("socket,id=chrtpm,path=/tmp/emulated_tpm-2/swtpm-sock".to_string());
        self
    }

    fn vga(&mut self) -> &mut Self {
        self.cmd.push("-vga".to_string());
        self.cmd.push("std".to_string());
        self
    }

    fn append<T: Into<String>>(&mut self, append: T) -> &mut Self {
        self.cmd.push("-append".to_string());
        self.cmd.push(append.into());
        self
    }

    fn virtio_gpu(&mut self) -> &mut Self {
        self.cmd.push("-device".to_string());
        self.cmd.push("virtio-gpu-pci".to_string());
        self
    }

    fn spawn(&self) -> Result<()> {
        let mut cmd = Command::new("qemu-system-x86_64");
        cmd.args(self.cmd.clone());
        self.nets.iter().for_each(|e| {
            cmd.args(e.to_string());
        });
        let mut args_iter = cmd.get_args();
        while let Some(arg) = args_iter.next() {
            if (arg.to_string_lossy().starts_with('-')) {
                print!("\t{}", arg.to_string_lossy());
                if let Some(next_arg) = args_iter.next() {
                    println!("{}", next_arg.to_string_lossy());
                }
            }
        }
        let mut child = cmd.spawn()?;
        child.wait()?;
        Ok(())
    }
}

fn main() -> Result<()> {
    QemuInstance::new("/home/rucoder/zd/eve/dist/amd64/current")
        .machine()
        .eve_serial("13471118009978")
        .cpu()
        .iommu()
        .ram(4096)
        .rtc()
        .serial()
        //.video("sdl")
        .bios_file("OVMF_CODE.fd", 0)
        .bios_file("OVMF_VARS.fd", 1)
        .drive("live.qcow2")
        .net(
            NetDevice::new("eth0")
                .port_forward(2222, 22)
                .mask("192.168.1.0/24")
                .dhcp_start("192.168.1.10"),
        )
        .net(
            NetDevice::new("eth1")
                .mask("192.168.2.0/24")
                .dhcp_start("192.168.2.10"),
        )
        //.tpm()
        .vga()
        .virtio_gpu()
        //.append("console=ttyS1")
        .spawn()?;
    Ok(())
}
