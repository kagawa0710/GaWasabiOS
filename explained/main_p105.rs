//! WASABI OS - 詳細解説版 main.rs (p105時点)
//! 
//! このファイルは学習用として、p105時点のmain.rsの全ての部分を詳細にコメントで解説しています。
//! 実際のビルドには使用されません。
//!
//! ## 進捗状況
//! - p72まで: 基本的なUEFI起動、画面制御、HLT命令
//! - p80まで: Bitmapトレイト、図形描画関数、効率的描画システム
//! - p83まで: 線描画アルゴリズム（Bresenham風）、グリッド・放射線描画
//! - p91まで: フォント描画システム、文字レンダリング、テキスト表示
//! - p97まで: 高度なテキスト描画、カーソル機能、改行処理、fmt::Write実装
//! - p105まで: メモリマップ機能、システムメモリ情報表示、メモリ管理基盤
//!
//! ## Git履歴での対応
//! - 68cbd50: first commit
//! - 87fdcc2: UEFI graphics output and CPU halt functionality (p72相当)
//! - ae29ca0: detailed code explanation file for learning
//! - 9de5a01: Bitmap trait and pixel-level graphics operations (p80相当)
//! - 7a25466: line drawing algorithm and complex graphics demo (p83相当)
//! - 1a8562b: font rendering system and bitmap text display (p91相当)
//! - f92b09d: advanced text rendering with cursor and formatting (p97相当)
//! - 現在: memory mapping functionality and system information display (p105相当)

// ============================================================================
// コンパイラ属性・インポート（p105での拡張）
// ============================================================================

#![no_std]   // 標準ライブラリを使わない（OS開発で必須）
#![no_main]  // 通常のmain関数を使わない（UEFIエントリポイント使用）
#![feature(offset_of)] // 構造体オフセット計算の実験的機能を有効化

use core::arch::asm;         // インラインアセンブリ（HLT命令用）
use core::cmp::min;          // 最小値計算（境界チェック用）
use core::fmt;               // フォーマット機能（継続）
use core::fmt::Write;        // 文字列書き込みトレイト（継続）
use core::mem::offset_of;    // 構造体メンバーのメモリ位置計算（メモリマップで重要）
use core::mem::size_of;      // 型のサイズ（バイト数）取得
use core::panic::PanicInfo;  // パニック時の情報
use core::ptr::null_mut;     // NULLポインタ作成
use core::writeln;           // 文字列書き込みマクロ（継続）

// ============================================================================
// 型エイリアス・基本定義（継続）
// ============================================================================

type EfiVoid = u8;    // UEFIの汎用ポインタ型
type EfiHandle = u64; // UEFIオブジェクトの識別子
type Result<T> = core::result::Result<T, &'static str>; // エラーハンドリング型

// UEFI GUID（128ビット一意識別子）
#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
struct EfiGuid {
    data0: u32,
    data1: u16,
    data2: u16,
    data3: [u8; 8],
}

// Graphics Output ProtocolのGUID（UEFI仕様で固定値）
const EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data0: 0x9042a9de,
    data1: 0x23dc,
    data2: 0x4a38,
    data3: [0x96, 0xfb, 0x7a, 0xde, 0xd0, 0x80, 0x51, 0x6a],
};

// UEFI関数の戻り値
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[must_use]
#[repr(u64)]
enum EfiStatus {
    Success = 0,
}

// ============================================================================
// メモリ管理関連の新機能（p105の新機能）
// ============================================================================

/// EFIメモリタイプ列挙型
/// 
/// UEFIファームウェアがシステムメモリをどのように分類しているかを示す
/// OSは使用可能なメモリ種類を判断するためにこの情報を使用する
#[repr(i64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum EfiMemoryType {
    RESERVED = 0,                    // 予約済み領域（使用不可）
    LOADER_CODE,                     // ローダーのコード領域
    LOADER_DATA,                     // ローダーのデータ領域
    BOOT_SERVICES_CODE,              // ブートサービスのコード
    BOOT_SERVICES_DATA,              // ブートサービスのデータ
    RUNTIME_SERVICES_CODE,           // ランタイムサービスのコード
    RUNTIME_SERVICES_DATA,           // ランタイムサービスのデータ
    CONVENTIONAL_MEMORY,             // 通常のメモリ（OSが自由に使用可能）
    UNUSABLE_MEMORY,                 // 使用不可能なメモリ
    ACPI_RECLAIM_MEMORY,            // ACPI用（回収可能）
    ACPI_MEMORY_NVS,                // ACPI用（不揮発性）
    MEMORY_MAPPED_IO,               // メモリマップドI/O
    MEMORY_MAPPED_IO_PORT_SPACE,    // メモリマップドI/Oポート
    PAL_CODE,                       // プロセッサ固有コード
    PERSISTENT_MEMORY,              // 持続メモリ
}

/// EFIメモリディスクリプタ
/// 
/// 各メモリ領域の詳細情報を格納する構造体
/// UEFIから取得される個別のメモリエントリの情報
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct EfiMemoryDescriptor {
    memory_type: EfiMemoryType,     // メモリの種類
    physical_start: u64,            // 物理アドレスの開始位置
    virtual_start: u64,             // 仮想アドレスの開始位置（通常は0）
    number_of_pages: u64,           // ページ数（1ページ = 4KB）
    attribute: u64,                 // メモリ属性（読み込み専用等のフラグ）
}

/// メモリマップ用バッファサイズ（32KB）
/// 
/// UEFIから取得するメモリマップ情報を格納するためのバッファサイズ
/// 通常のシステムでは数十〜数百のメモリエントリがあるため、
/// 十分な余裕をもって32KBを確保
const MEMORY_MAP_BUFFER_SIZE: usize = 0x8000;

/// メモリマップ保持構造体
/// 
/// UEFIから取得したメモリ情報を格納・管理する構造体
/// メモリマップのデータとメタ情報を一括管理
struct MemoryMapHolder {
    memory_map_buffer: [u8; MEMORY_MAP_BUFFER_SIZE],  // メモリマップデータの格納バッファ
    memory_map_size: usize,                           // 実際のマップサイズ（バイト数）
    map_key: usize,                                   // マップのキー値（UEFI内部で使用）
    descriptor_size: usize,                           // 各エントリのサイズ（バイト数）
    descriptor_version: u32,                          // ディスクリプタのバージョン
}

/// メモリマップイテレータ
/// 
/// メモリマップの各エントリを順次取得するためのイテレータ
/// Rustのfor文でメモリエントリを順次処理できるようにする
struct MemoryMapIterator<'a> {
    map: &'a MemoryMapHolder,  // メモリマップへの参照
    ofs: usize,                // 現在のオフセット位置（バイト単位）
}

/// Iterator トレイトの実装
/// 
/// for ループでメモリエントリを順次処理できるようにする
/// Rustの標準的なイテレータパターンを実装
impl<'a> Iterator for MemoryMapIterator<'a> {
    type Item = &'a EfiMemoryDescriptor;  // 返すアイテムの型
    
    fn next(&mut self) -> Option<&'a EfiMemoryDescriptor> {
        // Step 1: バッファの終端に達したかチェック
        if self.ofs >= self.map.memory_map_size {
            None  // 終了
        } else {
            // Step 2: 現在位置のメモリディスクリプタを取得
            // unsafeブロック: バイト配列を構造体ポインタに変換
            let e: &EfiMemoryDescriptor = unsafe {
                &*(self.map.memory_map_buffer.as_ptr().add(self.ofs) as *const EfiMemoryDescriptor)
            };
            
            // Step 3: 次のエントリ位置に移動
            self.ofs += self.map.descriptor_size;
            
            Some(e)  // エントリを返す
        }
    }
}

/// MemoryMapHolder の実装
impl MemoryMapHolder {
    /// 新しいメモリマップホルダーを作成
    /// 
    /// バッファを0で初期化し、サイズをバッファサイズで初期化
    /// const関数として定義することで、コンパイル時に初期化可能
    pub const fn new() -> MemoryMapHolder {
        MemoryMapHolder {
            memory_map_buffer: [0; MEMORY_MAP_BUFFER_SIZE],
            memory_map_size: MEMORY_MAP_BUFFER_SIZE,
            map_key: 0,
            descriptor_size: 0,
            descriptor_version: 0,
        }
    }
    
    /// イテレータを取得
    /// 
    /// for ループでメモリエントリを順次処理するため
    pub fn iter(&self) -> MemoryMapIterator {
        MemoryMapIterator { 
            map: self, 
            ofs: 0 
        }
    }
}

// ============================================================================
// UEFI Boot Services Table の拡張（p105の新機能）
// ============================================================================

/// EFI Boot Services Table
/// 
/// メモリマップ取得機能を追加した拡張版
/// UEFIの機能を呼び出すためのテーブル構造体
#[repr(C)]
struct EfiBootServicesTable {
    _reserved0: [u64; 7],           // 56バイトのスキップ領域（他の関数のスロット）
    
    // 新しく追加されたメモリマップ取得関数
    // UEFIの get_memory_map サービス関数へのポインタ
    get_memory_map: extern "win64" fn(
        memory_map_size: *mut usize,      // [入出力] マップサイズ
        memory_map: *mut u8,              // [出力] マップデータの格納先
        map_key: *mut usize,              // [出力] マップキー
        descriptor_size: *mut usize,      // [出力] 各エントリのサイズ
        descriptor_version: *mut u32,     // [出力] バージョン情報
    ) -> EfiStatus,
    
    _reserved1: [u64; 32],          // 256バイトのスキップ領域（他の関数のスロット）
    
    // 既存の locate_protocol 関数（継続）
    locate_protocol: extern "win64" fn(
        protocol: *const EfiGuid,
        registration: *mut EfiVoid,
        interface: *mut *mut EfiVoid,
    ) -> EfiStatus,
}

/// EfiBootServicesTable の便利メソッド実装
impl EfiBootServicesTable {
    /// メモリマップ取得の便利関数
    /// 
    /// UEFIの生の関数呼び出しをRust流にラップ
    /// エラーハンドリングと型安全性を向上
    fn get_memory_map(&self, map: &mut MemoryMapHolder) -> EfiStatus {
        (self.get_memory_map)(
            &mut map.memory_map_size,           // マップサイズ（入出力）
            map.memory_map_buffer.as_mut_ptr(), // バッファの開始位置
            &mut map.map_key,                   // マップキー（出力）
            &mut map.descriptor_size,           // エントリサイズ（出力）
            &mut map.descriptor_version,        // バージョン（出力）
        )
    }
}

// オフセット確認（構造体レイアウトの検証）
const _: () = assert!(offset_of!(EfiBootServicesTable, get_memory_map) == 56);
const _: () = assert!(offset_of!(EfiBootServicesTable, locate_protocol) == 320);

// ============================================================================
// UEFI System Table（継続）
// ============================================================================

#[repr(C)]
struct EfiSystemTable {
    _reserved0: [u64; 12],                           // 96バイトのスキップ
    pub boot_services: &'static EfiBootServicesTable, // Boot Servicesテーブルへの参照
}
const _: () = assert!(offset_of!(EfiSystemTable, boot_services) == 96);

// ============================================================================
// Graphics 関連構造体（継続）
// ============================================================================

#[repr(C)]
#[derive(Debug)]
struct EfiGraphicsOutputProtocolPixelInfo {
    pub version: u32,
    pub horizontal_resolution: u32,
    pub vertical_resolution: u32,
    _padding0: [u32; 5],
    pub pixels_per_scan_line: u32,
}
const _: () = assert!(size_of::<EfiGraphicsOutputProtocolPixelInfo>() == 36);

#[repr(C)]
#[derive(Debug)]
struct EfiGraphicsOutputProtocolMode<'a> {
    pub max_mode: u32,
    pub mode: u32,
    pub info: &'a EfiGraphicsOutputProtocolPixelInfo,
    pub size_of_info: u32,
    pub frame_buffer_base: usize,
    pub frame_buffer_size: usize,
}

#[repr(C)]
#[derive(Debug)]
struct EfiGraphicsOutputProtocol<'a> {
    reserved: [u64; 3],
    pub mode: &'a EfiGraphicsOutputProtocolMode<'a>,
}

/// Graphics プロトコル取得関数（継続）
fn locate_graphic_protocol<'a>(
    efi_system_table: &'a EfiSystemTable,
) -> Result<&'a EfiGraphicsOutputProtocol<'a>> {
    let mut efi_graphics_output_protocol = null_mut::<EfiGraphicsOutputProtocol>();
    let status = (efi_system_table.boot_services.locate_protocol)(
        &EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID,
        null_mut::<EfiVoid>(),
        &mut efi_graphics_output_protocol as *mut *mut EfiGraphicsOutputProtocol
            as *mut *mut EfiVoid,
    );
    if status != EfiStatus::Success {
        return Err("Failed to locate graphics output protocol");
    }
    Ok(unsafe { &*efi_graphics_output_protocol })
}

/// CPU制御関数（継続）
pub fn hlt() {
    unsafe {
        asm!("hlt");  // x86のHLT命令：CPUを低電力状態にして割り込み待ち
    }
}

// ============================================================================
// Bitmap トレイト・VRAM管理（継続）
// ============================================================================

trait Bitmap {
    fn bytes_per_pixel(&self) -> i64;
    fn pixels_per_scan_line(&self) -> i64;
    fn width(&self) -> i64;
    fn height(&self) -> i64;
    fn buf_mut(&mut self) -> *mut u8;

    /// # Safety
    ///
    /// Returned pointer is valid as long as the given coordinates are valid.
    /// which means that passing is_in_*_range tests.
    unsafe fn unchecked_pixel_at_mut(&mut self, x: i64, y: i64) -> *mut u32 {
        self.buf_mut()
            .add(((y * self.pixels_per_scan_line() + x) * self.bytes_per_pixel()) as usize)
            as *mut u32
    }
    fn pixel_at_mut(&mut self, x: i64, y: i64) -> Option<*mut u32> {
        if self.is_in_x_range(x) && self.is_in_y_range(y) {
            // SAFETY: (x, y) is always validated by the checks above.
            unsafe { Some(&mut *self.unchecked_pixel_at_mut(x, y)) }
        } else {
            None
        }
    }
    fn is_in_x_range(&self, px: i64) -> bool {
        0 <= px && px < min(self.width(), self.pixels_per_scan_line())
    }
    fn is_in_y_range(&self, py: i64) -> bool {
        0 <= py && py < self.height()
    }
}

#[derive(Clone, Copy)]
struct VramBefferInfo {
    buf: *mut u8,
    width: i64,
    height: i64,
    pixels_per_line: i64,
}

impl Bitmap for VramBefferInfo {
    fn bytes_per_pixel(&self) -> i64 {
        4
    }
    fn pixels_per_scan_line(&self) -> i64 {
        self.pixels_per_line
    }
    fn width(&self) -> i64 {
        self.width
    }
    fn height(&self) -> i64 {
        self.height
    }
    fn buf_mut(&mut self) -> *mut u8 {
        self.buf
    }
}

fn init_vram(efi_system_table: &EfiSystemTable) -> Result<VramBefferInfo> {
    let gp = locate_graphic_protocol(efi_system_table)?;

    Ok(VramBefferInfo {
        buf: gp.mode.frame_buffer_base as *mut u8,
        width: gp.mode.info.horizontal_resolution as i64,
        height: gp.mode.info.vertical_resolution as i64,
        pixels_per_line: gp.mode.info.pixels_per_scan_line as i64,
    })
}

// ============================================================================
// 基本描画関数群（継続）
// ============================================================================

/// # Safety
///
/// (x, y) must be a valid point in the buf.
unsafe fn unchecked_draw_point<T: Bitmap>(buf: &mut T, color: u32, x: i64, y: i64) {
    *buf.unchecked_pixel_at_mut(x, y) = color;
}

fn draw_point<T: Bitmap>(buf: &mut T, color: u32, x: i64, y: i64) -> Result<()> {
    unsafe {
        *(buf.pixel_at_mut(x, y).ok_or("Out of Range")?) = color;
    }
    Ok(())
}

fn fill_rect<T: Bitmap>(buf: &mut T, color: u32, px: i64, py: i64, w: i64, h: i64) -> Result<()> {
    if !buf.is_in_x_range(px)
        || !buf.is_in_y_range(py)
        || !buf.is_in_x_range(px + w - 1)
        || !buf.is_in_y_range(py + h - 1)
    {
        return Err("Out of Range");
    }
    for y in py..py + h {
        for x in px..px + w {
            unsafe {
                unchecked_draw_point(buf, color, x, y);
            }
        }
    }
    Ok(())
}

// ============================================================================
// 線描画アルゴリズム（継続）
// ============================================================================

fn calc_slope_point(da: i64, db: i64, ia: i64) -> Option<i64> {
    if da < db {
        None
    } else if da == 0 {
        Some(0)
    } else if (0..=da).contains(&ia) {
        Some((2 * db * ia + da) / da / 2)
    } else {
        None
    }
}

fn draw_line<T: Bitmap>(buf: &mut T, color: u32, x0: i64, y0: i64, x1: i64, y1: i64) -> Result<()> {
    if !buf.is_in_x_range(x0)
        || !buf.is_in_y_range(y0)
        || !buf.is_in_x_range(x1)
        || !buf.is_in_y_range(y1)
    {
        return Err("Out of Range");
    }
    let dx = (x1 - x0).abs();
    let dy = (y1 - y0).abs();
    let sx = (x1 - x0).signum();
    let sy = (y1 - y0).signum();
    if dx >= dy {
        for (rx, ry) in (0..dx).flat_map(|rx| calc_slope_point(dx, dy, rx).map(|ry| (rx, ry))) {
            draw_point(buf, color, x0 + rx * sx, y0 + ry * sy)?;
        }
    } else {
        for (rx, ry) in (0..dy).flat_map(|ry| calc_slope_point(dy, dx, ry).map(|rx| (rx, ry))) {
            draw_point(buf, color, x0 + rx * sx, y0 + ry * sy)?;
        }
    }
    Ok(())
}

// ============================================================================
// フォント描画システム（継続）
// ============================================================================

fn lookup_font(c: char) -> Option<[[char; 8]; 16]> {
    const FONT_SOURCE: &str = include_str!("font.txt");
    if let Ok(c) = u8::try_from(c) {
        let mut fi = FONT_SOURCE.split('\n');
        while let Some(line) = fi.next() {
            if let Some(line) = line.strip_prefix("0x") {
                if let Ok(idx) = u8::from_str_radix(line, 16) {
                    if idx != c {
                        continue;
                    }
                    let mut font = [['*'; 8]; 16];
                    for (y, line) in fi.clone().take(16).enumerate() {
                        for (x, c) in line.chars().enumerate() {
                            if let Some(e) = font[y].get_mut(x) {
                                *e = c;
                            }
                        }
                    }
                    return Some(font);
                }
            }
        }
    }
    None
}

fn draw_font_fg<T: Bitmap>(buf: &mut T, x: i64, y: i64, color: u32, c: char) {
    if let Some(font) = lookup_font(c) {
        for (dy, row) in font.iter().enumerate() {
            for (dx, pixel) in row.iter().enumerate() {
                let color = match pixel {
                    '*' => color,
                    _ => continue,
                };
                let _ = draw_point(buf, color, x + dx as i64, y + dy as i64);
            }
        }
    }
}

fn draw_str_fg<T: Bitmap>(buf: &mut T, x: i64, y: i64, color: u32, s: &str) {
    for (i, c) in s.chars().enumerate() {
        draw_font_fg(buf, x + i as i64 * 8, y, color, c);
    }
}

// ============================================================================
// 高度なテキスト描画システム（継続）
// ============================================================================

struct VramTextWriter<'a> {
    vram: &'a mut VramBefferInfo,
    cursor_x: i64,
    cursor_y: i64,
}
impl<'a> VramTextWriter<'a> {
    fn new(vram: &'a mut VramBefferInfo) -> Self {
        Self {
            vram,
            cursor_x: 0,
            cursor_y: 0,
        }
    }
}

impl fmt::Write for VramTextWriter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            if c == '\n' {
                self.cursor_x = 0;
                self.cursor_y += 16;
                continue;
            }
            draw_font_fg(self.vram, self.cursor_x, self.cursor_y, 0xffffff, c);
            self.cursor_x += 8;
        }
        Ok(())
    }
}

// ============================================================================
// メイン関数：メモリマップ機能のデモンストレーション（p105の新機能）
// ============================================================================

#[no_mangle]
// The entry point for the EFI application
fn efi_main(_image_handle: EfiHandle, efi_system_table: &EfiSystemTable) {
    // Step 1: VRAM初期化
    let mut vram = init_vram(efi_system_table).expect("init_vram failed");
    let vw = vram.width;
    let vh = vram.height;
    
    // === 基本描画デモ（継続） ===
    
    // 背景を黒で塗りつぶし
    fill_rect(&mut vram, 0x000000, 0, 0, vw, vh).expect("fill_rect failed");
    
    // カラフルな矩形（テスト用）
    fill_rect(&mut vram, 0xff0000, 32, 32, 32, 32).expect("fill_rect failed");
    fill_rect(&mut vram, 0x00ff00, 64, 64, 64, 64).expect("fill_rect failed");
    fill_rect(&mut vram, 0x0000ff, 128, 128, 128, 128).expect("fill_rect failed");
    
    // グラデーション対角線
    for i in 0..256 {
        let _ = draw_point(&mut vram, 0x010101 * i as u32, i, i);
    }
    
    // === 線描画デモ（継続） ===
    
    let grid_size: i64 = 32;
    let rect_size: i64 = grid_size * 8;
    for i in (0..=rect_size).step_by(grid_size as usize) {
        let _ = draw_line(&mut vram, 0xff0000, 0, i, rect_size, i);
        let _ = draw_line(&mut vram, 0xff0000, i, 0, i, rect_size);
    }
    let cx = rect_size / 2;
    let cy = rect_size / 2;
    for i in (0..=rect_size).step_by(grid_size as usize) {
        let _ = draw_line(&mut vram, 0xffff00, cx, cy, 0, i);
        let _ = draw_line(&mut vram, 0x00ffff, cx, cy, i, 0);
        let _ = draw_line(&mut vram, 0xff00ff, cx, cy, rect_size, i);
        let _ = draw_line(&mut vram, 0xffffff, cx, cy, i, rect_size);
    }
    
    // === フォント描画デモ（継続） ===
    
    for (i, c) in "ABCDEF".chars().enumerate() {
        draw_font_fg(&mut vram, i as i64 * 16 + 256, i as i64 * 16, 0xffffff, c)
    }
    draw_str_fg(&mut vram, 256, 256, 0xffffff, "Hello, world!");
    
    // === 高度なテキスト描画デモ（継続） ===
    
    let mut w = VramTextWriter::new(&mut vram);
    for i in 0..4 {
        writeln!(w, "i = {i}").unwrap();
    }
    
    // === メモリマップ機能デモ（p105の新機能） ===
    
    // Step 1: メモリマップホルダーを作成
    let mut memory_map = MemoryMapHolder::new();
    
    // Step 2: UEFIからメモリマップを取得
    let status = efi_system_table
        .boot_services
        .get_memory_map(&mut memory_map);
    
    // Step 3: 取得結果を表示
    writeln!(w, "{status:?}").unwrap();
    
    // Step 4: 各メモリエントリを順次表示
    // イテレータパターンを使用してメモリエントリを順次処理
    for e in memory_map.iter() {
        writeln!(w, "{e:?}").unwrap();
    }
    
    // 無限ループで画面を保持
    loop {
        hlt()
    }
}

// ============================================================================
// パニックハンドラー（継続）
// ============================================================================

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        hlt()
    }
}

// ============================================================================
// アーキテクチャの進化（p72 → p80 → p83 → p91 → p97 → p105）
// ============================================================================
/*
【p72時点】
- 基本的なUEFI起動
- 直接VRAMアクセス  
- 単純なピクセル操作

【p80時点】
- Bitmapトレイトによる抽象化
- 安全性と効率性を両立した描画システム
- 再利用可能な図形描画関数（点・矩形）

【p83時点】
- 線描画アルゴリズムの実装（Bresenham風）
- 複雑なグラフィックパターン（グリッド・放射線）
- より高度な2D描画の基盤完成

【p91時点】
- フォント描画システムの実装
- 外部ファイル（font.txt）からのフォントデータ読み込み
- 文字レンダリング機能
- テキスト表示の基礎完成

【p97時点】
- 高度なテキスト描画システム
- カーソル位置管理機能
- 改行処理（\n）の実装
- fmt::Writeトレイト実装による標準的なテキスト出力インターフェース
- writeln!マクロ対応とformatted出力機能

【p105時点】
- メモリマップ機能の実装
- EfiMemoryType列挙型による メモリ領域分類
- EfiMemoryDescriptor構造体でのメモリ詳細情報管理
- MemoryMapHolder/Iteratorパターンによるメモリ情報の順次処理
- UEFIからのシステムメモリ情報取得・表示
- get_memory_map関数の追加とEfiBootServicesTable拡張
- メモリ管理・アロケータ実装の基盤完成

【技術的進歩（p105）】
1. **メモリマップ取得**: UEFIからのシステムメモリ情報取得
2. **メモリ分類システム**: 各メモリ領域の用途と属性の把握
3. **イテレータパターン**: Rust流のメモリエントリ順次処理
4. **構造体オフセット検証**: コンパイル時の構造体レイアウト確認
5. **メモリ管理基盤**: 将来のアロケータ実装のための基礎

【メモリマップシステムの仕組み】
- EfiMemoryType: メモリ領域の種類を分類（使用可能かどうかの判断）
- EfiMemoryDescriptor: 各メモリ領域の詳細情報（アドレス、サイズ、属性）
- MemoryMapHolder: UEFIから取得したメモリ情報の管理
- MemoryMapIterator: メモリエントリの順次アクセス

【表示されるメモリ情報の例】
```
Success
EfiMemoryDescriptor { memory_type: BOOT_SERVICES_DATA, physical_start: 0x0, ... }
EfiMemoryDescriptor { memory_type: CONVENTIONAL_MEMORY, physical_start: 0x1000, ... }
...
```

【次のステップ予想】
- メモリアロケータの実装（空きメモリからの動的割り当て）
- 仮想メモリ管理システム
- ページテーブル管理
- プロセス管理用メモリ保護
- より高度なメモリ最適化

これでOSが使用可能なメモリ領域を把握する基盤が完成し、
後の章でのメモリ管理機能の土台が整いました。

【学習ポイント】
- UEFIファームウェアとOSの連携方法
- システムメモリの構造と分類
- Rustでの低レベルメモリ操作
- イテレータパターンによる安全なデータ処理
- 構造体レイアウトの厳密な管理

これは現代的なOSに欠かせないメモリ管理機能の第一歩です。
*/