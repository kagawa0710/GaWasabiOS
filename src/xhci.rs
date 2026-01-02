extern crate alloc;

use crate::allocator::ALLOCATOR;
use crate::bits::extract_bits;
use crate::executor::spawn_global;
use crate::info;
use crate::mmio::Mmio;
use crate::mutex::Mutex;
use crate::pci::BarMem64;
use crate::pci::BusDeviceFunction;
use crate::pci::Pci;
use crate::pci::VendorDeviceId;
use crate::result::Result;
use crate::volatile::Volatile;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::alloc::Layout;
use core::cmp::max;
use core::marker::PhantomPinned;
use core::mem::size_of;
use core::mem::MaybeUninit;
use core::pin::Pin;
use core::slice;

// ============================================================
// Capability Registers (xHCI spec 5.3)
// ============================================================
#[repr(C)]
struct CapabilityRegisters {
    caplength: Volatile<u8>,
    reserved: Volatile<u8>,
    version: Volatile<u16>,
    hcsparams1: Volatile<u32>,
    hcsparams2: Volatile<u32>,
    hcsparams3: Volatile<u32>,
    hccparams1: Volatile<u32>,
    dboff: Volatile<u32>,
    rtsoff: Volatile<u32>,
    hccparams2: Volatile<u32>,
}
const _: () = assert!(size_of::<CapabilityRegisters>() == 0x20);

impl CapabilityRegisters {
    fn caplength(&self) -> usize {
        self.caplength.read() as usize
    }
    fn rtsoff(&self) -> usize {
        self.rtsoff.read() as usize
    }
    fn num_of_device_slots(&self) -> usize {
        extract_bits(self.hcsparams1.read(), 0, 8) as usize
    }
    fn num_scratchpad_bufs(&self) -> usize {
        (extract_bits(self.hcsparams2.read(), 21, 5) << 5
            | extract_bits(self.hcsparams2.read(), 27, 5)) as usize
    }
}

// ============================================================
// Operational Registers (xHCI spec 5.4)
// ============================================================
#[repr(C)]
struct OperationalRegisters {
    usbcmd: Volatile<u32>,
    usbsts: Volatile<u32>,
    pagesize: Volatile<u32>,
    rsvdz1: [u32; 2],
    dnctrl: Volatile<u32>,
    crcr: Volatile<u64>,
    rsvdz2: [u64; 2],
    dcbaap: Volatile<*mut RawDeviceContextBaseAddressArray>,
    config: Volatile<u64>,
}
const _: () = assert!(size_of::<OperationalRegisters>() == 0x40);

impl OperationalRegisters {
    fn usbsts(&self) -> u32 {
        self.usbsts.read()
    }
    fn page_size(&self) -> Result<usize> {
        let page_size_bits = self.pagesize.read() & 0xFFFF;
        if page_size_bits.count_ones() != 1 {
            return Err("PAGE_SIZE has multiple bits set");
        }
        let page_size_shift = page_size_bits.trailing_zeros();
        Ok(1 << (page_size_shift + 12))
    }
}

// ============================================================
// Runtime Registers (xHCI spec 5.5)
// ============================================================
#[repr(C)]
struct InterrupterRegisterSet {
    management: u32,
    moderation: u32,
    erst_size: u32,
    rsvdp: u32,
    erst_base: u64,
    erdp: u64,
}
const _: () = assert!(size_of::<InterrupterRegisterSet>() == 0x20);

#[repr(C)]
struct RuntimeRegisters {
    mfindex: Volatile<u32>,
    rsvdz: [u32; 7],
    irs: [InterrupterRegisterSet; 1024],
}
const _: () = assert!(size_of::<RuntimeRegisters>() == 0x8020);

impl RuntimeRegisters {
    fn mfindex(&self) -> u32 {
        self.mfindex.read()
    }
}

// ============================================================
// Device Context Structures
// ============================================================
#[repr(C, align(64))]
struct RawDeviceContextBaseAddressArray {
    scratchpad_table_ptr: *const *const u8,
    context: [u64; 255],
    _pinned: PhantomPinned,
}
const _: () = assert!(size_of::<RawDeviceContextBaseAddressArray>() == 2048);

impl RawDeviceContextBaseAddressArray {
    fn new() -> Self {
        unsafe { MaybeUninit::zeroed().assume_init() }
    }
}

#[repr(C, align(32))]
#[derive(Default, Clone)]
struct EndpointContext {
    data: [u32; 2],
    tr_dequeue_ptr: Volatile<u64>,
    average_trb_length: u16,
    max_esit_payload_low: u16,
    _reserved: [u32; 3],
}
const _: () = assert!(size_of::<EndpointContext>() == 0x20);

#[repr(C, align(32))]
#[derive(Default, Clone)]
struct DeviceContext {
    slot_ctx: [u32; 8],
    ep_ctx: [EndpointContext; 2 * 15 + 1],
    _pinned: PhantomPinned,
}
const _: () = assert!(size_of::<DeviceContext>() == 0x400);

#[repr(C, align(4096))]
#[derive(Default, Clone)]
struct OutputContext {
    device_ctx: DeviceContext,
    _pinned: PhantomPinned,
}
const _: () = assert!(size_of::<OutputContext>() <= 4096);

// ============================================================
// Scratchpad Buffers
// ============================================================
struct ScratchpadBuffers {
    table: Pin<Box<[*const u8]>>,
    _bufs: Vec<Pin<Box<[u8]>>>,
}

impl ScratchpadBuffers {
    fn alloc(cap_regs: &CapabilityRegisters, op_regs: &OperationalRegisters) -> Result<Self> {
        let page_size = op_regs.page_size()?;
        info!("xhci: page_size = {page_size}");

        let num_scratchpad_bufs = cap_regs.num_scratchpad_bufs();
        info!("xhci: num_scratchpad_bufs = {num_scratchpad_bufs}");

        let num_scratchpad_bufs = max(num_scratchpad_bufs, 1);

        let mut bufs: Vec<Pin<Box<[u8]>>> = Vec::new();
        let mut table_vec: Vec<*const u8> = Vec::new();

        for _ in 0..num_scratchpad_bufs {
            let buf = ALLOCATOR.alloc_with_options(
                Layout::from_size_align(page_size, page_size)
                    .map_err(|_| "Failed to allocate scratchpad buffer")?,
            );
            let buf_slice = unsafe { slice::from_raw_parts_mut(buf, page_size) };
            buf_slice.fill(0);
            table_vec.push(buf as *const u8);
            bufs.push(Pin::new(unsafe { Box::from_raw(buf_slice as *mut [u8]) }));
        }

        let table: Box<[*const u8]> = table_vec.into_boxed_slice();
        let table = Pin::new(table);

        info!("xhci: Scratchpad buffers allocated");

        Ok(Self { table, _bufs: bufs })
    }
}

// ============================================================
// Device Context Base Address Array
// ============================================================
struct DeviceContextBaseAddressArray {
    _inner: Pin<Box<RawDeviceContextBaseAddressArray>>,
    _context: [Option<Pin<Box<OutputContext>>>; 255],
    _scratchpad_buffers: ScratchpadBuffers,
}

impl DeviceContextBaseAddressArray {
    fn new(scratchpad_buffers: ScratchpadBuffers) -> Self {
        let context: [Option<Pin<Box<OutputContext>>>; 255] = {
            const NONE: Option<Pin<Box<OutputContext>>> = None;
            [NONE; 255]
        };

        let mut inner = Box::pin(RawDeviceContextBaseAddressArray::new());

        // Set scratchpad table pointer
        unsafe {
            let inner_mut = Pin::get_unchecked_mut(inner.as_mut());
            inner_mut.scratchpad_table_ptr = scratchpad_buffers.table.as_ptr();
        }

        info!("xhci: DeviceContextBaseAddressArray created");

        Self {
            _inner: inner,
            _context: context,
            _scratchpad_buffers: scratchpad_buffers,
        }
    }
}

// ============================================================
// Controller
// ============================================================
struct Controller {
    cap_regs: Mmio<CapabilityRegisters>,
    op_regs: Mmio<OperationalRegisters>,
    rt_regs: Mmio<RuntimeRegisters>,
    _device_context_base_array: Mutex<DeviceContextBaseAddressArray>,
}

impl Controller {
    fn new(
        cap_regs: Mmio<CapabilityRegisters>,
        op_regs: Mmio<OperationalRegisters>,
        rt_regs: Mmio<RuntimeRegisters>,
        device_context_base_array: DeviceContextBaseAddressArray,
    ) -> Self {
        Self {
            cap_regs,
            op_regs,
            rt_regs,
            _device_context_base_array: Mutex::new(device_context_base_array),
        }
    }
}

// ============================================================
// PCI xHCI Driver
// ============================================================
pub struct PciXhciDriver {}

impl PciXhciDriver {
    const VDI_LIST: [VendorDeviceId; 3] = [
        VendorDeviceId {
            vendor: 0x1b36,
            device: 0x000d,
        },
        VendorDeviceId {
            vendor: 0x8086,
            device: 0x31a8,
        },
        VendorDeviceId {
            vendor: 0x8086,
            device: 0x02ed,
        },
    ];

    pub fn supports(vp: VendorDeviceId) -> bool {
        Self::VDI_LIST.contains(&vp)
    }

    fn setup_xhc_registers(
        bar0: &BarMem64,
    ) -> Result<(
        Mmio<CapabilityRegisters>,
        Mmio<OperationalRegisters>,
        Mmio<RuntimeRegisters>,
    )> {
        let cap_regs = unsafe { Mmio::from_raw(bar0.addr() as *mut CapabilityRegisters) };
        let op_regs = unsafe {
            Mmio::from_raw(
                bar0.addr().add(cap_regs.as_ref().caplength()) as *mut OperationalRegisters
            )
        };
        let rt_regs = unsafe {
            Mmio::from_raw(bar0.addr().add(cap_regs.as_ref().rtsoff()) as *mut RuntimeRegisters)
        };

        Ok((cap_regs, op_regs, rt_regs))
    }

    pub fn attach(pci: &Pci, bdf: BusDeviceFunction) -> Result<()> {
        info!("xhci: Found at {bdf:?}");
        pci.disable_interrupt(bdf)?;
        pci.enable_bus_master(bdf)?;

        let bar0 = pci.try_bar0_mem64(bdf)?;
        bar0.disable_cache();
        info!("xhci: {bar0:?}");

        let (cap_regs, op_regs, rt_regs) = Self::setup_xhc_registers(&bar0)?;

        // Allocate scratchpad buffers
        let scratchpad_buffers = ScratchpadBuffers::alloc(cap_regs.as_ref(), op_regs.as_ref())?;

        // Create device context base address array
        let device_context_base_array = DeviceContextBaseAddressArray::new(scratchpad_buffers);

        // Create controller
        let xhc = Controller::new(cap_regs, op_regs, rt_regs, device_context_base_array);

        // Spawn the run task
        spawn_global(Self::run(xhc));

        Ok(())
    }

    async fn run(xhc: Controller) -> Result<()> {
        info!(
            "xhci: cap_regs.MaxSlots = {}",
            xhc.cap_regs.as_ref().num_of_device_slots()
        );
        info!("xhci: op_regs.USBSTS = {}", xhc.op_regs.as_ref().usbsts());
        info!("xhci: rt_regs.MFINDEX = {}", xhc.rt_regs.as_ref().mfindex());

        Ok(())
    }
}
