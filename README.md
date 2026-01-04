
## 実行
‘cargo run‘

## 手動実行
### build
cargo build --target x86_64-unknown-uefi 

### build後のファイルのコピー
cp target/x86_64-unknown-uefi/debug/wasabi.efi mnt/EFI/BOOT/BOOTX64.EFI

### QEMUの実行
qemu-system-x86_64 -bios third_party/ovmf/RELEASEX64_OVMF.fd -drive format=raw,file=fat:rw:mnt 

## 参照
- https://www.usb.org/defined-class-codes
- https://www.intel.com/content/dam/www/public/us/en/documents/technical-specifications/extensible-host-controler-interface-usb-xhci.pdf
