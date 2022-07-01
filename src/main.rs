use anyhow::{Context, Result};
use core::time;
use std::thread::{self, JoinHandle};
use std::{
    fs,
    io::{BufRead, BufReader},
    ops::Add,
    process::{Child, Command, Stdio},
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
    eve_serial: String,
}

impl QemuInstance {
    fn new<T: Into<String>>(base_path: T, serial: T) -> Self {
        Self {
            cmd: Vec::new(),
            nets: Vec::new(),
            base_path: base_path.into(),
            eve_serial: serial.into(),
        }
    }
    fn machine(&mut self) -> &mut Self {
        self.cmd.push("-machine".to_string());
        self.cmd.push(
            "q35,accel=kvm,smm=on,usb=off,dump-guest-core=off,kernel-irqchip=split".to_string(),
        );
        self
        //,accel=kvm
    }
    fn cpu(&mut self) -> &mut Self {
        self.cmd.push("-cpu".to_string());
        self.cmd.push("host".to_string());
        //self.cmd.push("SandyBridge".to_string());

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
    // fn eve_serial<T: Into<String>>(&mut cmd: Command, serial: T) -> &mut Self {
    //     self.cmd.push("-smbios".to_string());
    //     self.cmd.push(format!("type=1,serial={}", serial.into()));
    //     self
    // }
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
        // self.cmd.push(format!(
        //     "if=pflash,format=raw,unit={},readonly=on,file=/usr/share/OVMF/{}",
        //     segment,
        //     file.into()
        // ));
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
        self.cmd.push(format!(
            "socket,id=chrtpm,path=./tpms/{}/swtpm-sock",
            self.eve_serial
        ));
        self
    }

    fn vga(&mut self, gui: bool) -> &mut Self {
        if gui {
            self.cmd.push("-vga".to_string());
            self.cmd.push("std".to_string());
        }
        else {
            self.cmd.push("-nographic".to_string());
        }
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

    fn gdb(&mut self) -> &mut Self {
        self.cmd.push("-s".to_string());
        self.cmd.push("-S".to_string());
        self
    }

    fn uefi_debug_log(&mut self) -> &mut Self {
        //-debugcon file:debug.log -global isa-debugcon.iobase=0x402
        self.cmd.push("-debugcon".to_string());
        self.cmd.push("file:debug.log".to_string());
        self.cmd.push("-global".to_string());
        self.cmd.push("isa-debugcon.iobase=0x402".to_string());
        self
    }

    fn spawn(&self, dry_run: bool) -> Result<()> {
        let mut cmd = Command::new("qemu-system-x86_64");
        cmd.args(self.cmd.clone());
        self.nets.iter().for_each(|e| {
            cmd.args(e.to_string());
        });

        cmd.arg("-smbios");
        cmd.arg(format!("type=1,serial={}", &self.eve_serial));

        let mut args_iter = cmd.get_args();
        while let Some(arg) = args_iter.next() {
            if (arg.to_string_lossy().starts_with('-')) {
                print!("\t{}", arg.to_string_lossy());
                if let Some(next_arg) = args_iter.next() {
                    println!(" {}", next_arg.to_string_lossy());
                }
            }
        }
        if !dry_run {
            run_swtpm(&self.eve_serial)?;

            let mut child = cmd.spawn().with_context(|| "Couldn't spawn QEMU")?;
            child.wait()?;
        }
        Ok(())
    }
}

fn run_process_bg(cmd: &mut Command) -> Result<JoinHandle<i32>> {
    let mut child = cmd
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(anyhow::Error::from)
        .with_context(|| {
            format!(
                "Cannot run {} {}",
                cmd.get_program().to_string_lossy(),
                cmd.get_args()
                    .map(|e| e.to_string_lossy())
                    .fold(String::new(), |mut acc, e| {
                        acc.push_str(&e);
                        acc
                    })
            )
        })?;

    let handle = thread::spawn(move || {
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let mut f = BufReader::new(stdout);
        let mut fe = BufReader::new(stderr);

        loop {
            match child.try_wait() {
                Ok(None) => {
                    let mut buf = String::new();
                    //println!("we are here");
                    // match f.read_line(&mut buf) {
                    //     Ok(0) => {
                    //         println!("EOF -->> STDOUT");
                    //         break;
                    //     }
                    //     Ok(_) => {
                    //         print!("[TPM]: {}", buf);
                    //     }
                    //     Err(e) => println!("an error!: {:?}", e),
                    // }
                    match fe.read_line(&mut buf) {
                        Ok(0) => {
                            println!("EOF -->> STDERR");
                            break;
                        }
                        Ok(_) => {
                            //print!("[TPM]: {}", buf);
                        }
                        Err(e) => println!("an error!: {:?}", e),
                    }
                }
                Ok(Some(exit_status)) => {
                    println!("Process exited with {}", exit_status);
                    break;
                }
                Err(err) => {
                    println!("Process exited with error {}", err);
                    break;
                }
            }
        }
        0
    });
    Ok(handle)
}

fn run_swtpm<T: Into<String>>(serial_number: T) -> Result<JoinHandle<i32>> {
    let mut cmd = Command::new("swtpm");
    let tpm_state_path = format!("./tpms/{}", serial_number.into());
    fs::create_dir_all(&tpm_state_path)?;
    let args = format!(
        "socket --tpmstate dir={} --ctrl type=unixio,path={}/swtpm-sock --log level=20 --tpm2 -t",
        &tpm_state_path, &tpm_state_path
    );
    cmd.args(args.split_ascii_whitespace());

    println!("starting swtpm: {}", args);
    run_process_bg(&mut cmd)
}

fn run_dmesg() {
    let mut cmd = Command::new("dmesg");
    run_process_bg(&mut cmd).unwrap().join();
}

fn main() -> Result<()> {
    //run_dmesg();
    //let tpm = run_swtpm("134711180099780011")?;
    //thread::sleep(time::Duration::from_secs(2));

    QemuInstance::new("/home/rucoder/zd/grub/eve/dist/amd64/current", "Mike-0003")
        .machine()
        .cpu()
        .iommu()
        .ram(512)
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
        .tpm()
        .vga(false)
        .uefi_debug_log()
        //.gdb()
        //.virtio_gpu()
        //.append("console=ttyS1")
        .spawn(false)?;

    println!("Exiting vm-runner...");
    //tpm.join().unwrap();
    Ok(())
}
