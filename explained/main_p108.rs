//! WASABI OS - 詳細解説版 main.rs (p108時点)
//! 
//! このファイルは学習用として、p108時点のmain.rsの全ての部分を詳細にコメントで解説しています。
//! 実際のビルドには使用されません。
//!
//! ## 進捗状況
//! - p72まで: 基本的なUEFI起動、画面制御、HLT命令
//! - p80まで: Bitmapトレイト、図形描画関数、効率的描画システム
//! - p83まで: 線描画アルゴリズム（Bresenham風）、グリッド・放射線描画
//! - p91まで: フォント描画システム、文字レンダリング、テキスト表示
//! - p97まで: 高度なテキスト描画、カーソル機能、改行処理、fmt::Write実装
//! - p105まで: メモリマップ機能、システムメモリ情報表示、メモリ管理基盤
//! - p108まで: コード整理、draw_test_pattern関数の分離、構造化改善
//!
//! ## Git履歴での対応
//! - 68cbd50: first commit
//! - 87fdcc2: UEFI graphics output and CPU halt functionality (p72相当)
//! - ae29ca0: detailed code explanation file for learning
//! - 9de5a01: Bitmap trait and pixel-level graphics operations (p80相当)
//! - 7a25466: line drawing algorithm and complex graphics demo (p83相当)
//! - 1a8562b: font rendering system and bitmap text display (p91相当)
//! - f92b09d: advanced text rendering with cursor and formatting (p97相当)
//! - 4563eeb: memory mapping functionality and system information display (p105相当)
//! - 現在: code refactoring and draw_test_pattern function extraction (p108相当)

// ============================================================================
// コンパイラ属性・インポート（継続）
// ============================================================================

#![no_std]   // 標準ライブラリを使わない（OS開発で必須）
#![no_main]  // 通常のmain関数を使わない（UEFIエントリポイント使用）
#![feature(offset_of)] // 構造体オフセット計算の実験的機能を有効化

// インラインアセンブリを使うための宣言
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
// メモリ管理関連（継続）
// ============================================================================

/// EFIメモリタイプ列挙型
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
const MEMORY_MAP_BUFFER_SIZE: usize = 0x8000;

/// メモリマップ保持構造体
struct MemoryMapHolder {
    memory_map_buffer: [u8; MEMORY_MAP_BUFFER_SIZE],  // メモリマップデータの格納バッファ
    memory_map_size: usize,                           // 実際のマップサイズ（バイト数）
    map_key: usize,                                   // マップのキー値（UEFI内部で使用）
    descriptor_size: usize,                           // 各エントリのサイズ（バイト数）
    descriptor_version: u32,                          // ディスクリプタのバージョン
}

/// メモリマップイテレータ
struct MemoryMapIterator<'a> {
    map: &'a MemoryMapHolder,  // メモリマップへの参照
    ofs: usize,                // 現在のオフセット位置（バイト単位）
}

/// Iterator トレイトの実装
impl<'a> Iterator for MemoryMapIterator<'a> {
    type Item = &'a EfiMemoryDescriptor;  // 返すアイテムの型
    
    fn next(&mut self) -> Option<&'a EfiMemoryDescriptor> {
        if self.ofs >= self.map.memory_map_size {
            None  // 終了
        } else {
            let e: &EfiMemoryDescriptor = unsafe {
                &*(self.map.memory_map_buffer.as_ptr().add(self.ofs) as *const EfiMemoryDescriptor)
            };
            self.ofs += self.map.descriptor_size;
            Some(e)  // エントリを返す
        }
    }
}

/// MemoryMapHolder の実装
impl MemoryMapHolder {
    /// 新しいメモリマップホルダーを作成
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
    pub fn iter(&self) -> MemoryMapIterator {
        MemoryMapIterator { map: self, ofs: 0 }
    }
}

// ============================================================================
// UEFI Boot Services Table（継続）
// ============================================================================

/// EFI Boot Services Table
#[repr(C)]
struct EfiBootServicesTable {
    _reserved0: [u64; 7],           // 56バイトのスキップ領域（他の関数のスロット）
    
    // メモリマップ取得関数
    get_memory_map: extern "win64" fn(
        memory_map_size: *mut usize,      // [入出力] マップサイズ
        memory_map: *mut u8,              // [出力] マップデータの格納先
        map_key: *mut usize,              // [出力] マップキー
        descriptor_size: *mut usize,      // [出力] 各エントリのサイズ
        descriptor_version: *mut u32,     // [出力] バージョン情報
    ) -> EfiStatus,
    
    _reserved1: [u64; 32],          // 256バイトのスキップ領域（他の関数のスロット）
    
    // locate_protocol 関数（継続）
    locate_protocol: extern "win64" fn(
        protocol: *const EfiGuid,
        registration: *mut EfiVoid,
        interface: *mut *mut EfiVoid,
    ) -> EfiStatus,
}

/// EfiBootServicesTable の便利メソッド実装
impl EfiBootServicesTable {
    /// メモリマップ取得の便利関数
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
// メイン関数：コード整理後の簡潔版（p108の改善）
// ============================================================================

#[no_mangle]
// The entry point for the EFI application(仕様でEFIアプリケーションのエントリポイントはefi_mainとなっている)
fn efi_main(_image_handle: EfiHandle, efi_system_table: &EfiSystemTable) {
    // Step 1: VRAM初期化
    let mut vram = init_vram(efi_system_table).expect("init_vram failed");
    let vw = vram.width;
    let vh = vram.height;
    
    // Step 2: 画面をクリア（黒で塗りつぶし）
    fill_rect(&mut vram, 0x000000, 0, 0, vw, vh).expect("fill_rect failed");
    
    // Step 3: テストパターンを描画（p108で新しく分離された関数）
    draw_test_pattern(&mut vram);
    
    // Step 4: テキストライターを作成してテキスト表示
    let mut w = VramTextWriter::new(&mut vram);
    for i in 0..4 {
        writeln!(w, "i = {i}").unwrap();
    }
    
    // Step 5: メモリマップ情報を取得・表示
    let mut memory_map = MemoryMapHolder::new();
    let status = efi_system_table
        .boot_services
        .get_memory_map(&mut memory_map);
    writeln!(w, "{status:?}").unwrap();
    
    // Step 6: 使用可能メモリ（CONVENTIONAL_MEMORY）のみを集計・表示
    let mut total_memory_pages = 0;
    for e in memory_map.iter() {
        if e.memory_type != EfiMemoryType::CONVENTIONAL_MEMORY {
            continue;  // CONVENTIONAL_MEMORY以外はスキップ
        }
        total_memory_pages += e.number_of_pages;  // ページ数を累積
        writeln!(w, "{e:?}").unwrap();           // エントリの詳細を表示
    }
    
    // Step 7: 合計メモリサイズをMiB単位で計算・表示
    let total_memory_size_mib = total_memory_pages * 4096 / 1024 / 1024;
    writeln!(
        w,
        "Total: {total_memory_pages} pages = {total_memory_size_mib} MiB",
    ).unwrap();
    
    // 無限ループで画面を保持
    loop {
        hlt()
    }
}

// ============================================================================
// テストパターン描画関数（p108の新機能）
// ============================================================================

/// 統合的なテストパターン描画関数
/// 
/// 以前はメイン関数内に散らばっていた描画コードを
/// 1つの関数にまとめて整理（p106-p108のリファクタリング）
fn draw_test_pattern<T: Bitmap>(buf: &mut T) {
    // === パラメータ設定 ===
    let w = 128;                              // テストパターンの幅
    let left = buf.width() - w - 1;          // 画面右側に配置
    let colors = [0x000000, 0xff0000, 0x00ff00, 0x0000ff]; // 黒、赤、緑、青
    let h = 64;                              // 各色ブロックの高さ
    
    // === カラーブロックの描画 ===
    for (i, c) in colors.iter().enumerate() {
        let y = i as i64 * h;  // Y座標を計算
        
        // 通常色のブロックを描画
        fill_rect(buf, *c, left, y, h, h).expect("fill_rect failed");
        
        // 反転色のブロックを描画（ビット反転で補色を生成）
        fill_rect(buf, !*c, left + h, y, h, h).expect("fill_rect failed");
    }
    
    // === 線描画パターン ===
    // 四角形の四隅を定義
    let points = [(0,0), (0,w), (w,0), (w,w)];
    
    // 全ての点から全ての点に線を引く（完全グラフ）
    for (x0, y0) in points.iter() {
        for (x1, y1) in points.iter() {
            // 白い線で接続
            let _ = draw_line(buf, 0xffffff, left + *x0, *y0, left + *x1, *y1);
        }
    }
    
    // === フォントテストの描画 ===
    // 数字の文字列をテスト
    draw_str_fg(buf, left, h * colors.len() as i64, 0x00ff00, "0123456789");
    
    // アルファベットの文字列をテスト
    draw_str_fg(buf, left, h * colors.len() as i64 + 16, 0x00ff00, "ABCDEF");
}

// ============================================================================
// パニックハンドラー（継続）
// ============================================================================

/// panic!()が呼ばれたときの処理
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        hlt()
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
// アーキテクチャの進化（p72 → p80 → p83 → p91 → p97 → p105 → p108）
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

【p108時点】
- コード整理とリファクタリング
- draw_test_pattern関数の分離
- メイン関数の簡潔化
- 描画デモコードの統合
- 保守性とコードの可読性向上

【技術的進歩（p108）】
1. **関数分離**: 散らばった描画コードを1つの関数にまとめる
2. **コード整理**: メイン関数の簡潔化と責任の明確化
3. **保守性向上**: 機能ごとの分離により修正・拡張が容易
4. **テストパターン統合**: 複数の描画テストを統一された形式で実行

【リファクタリングの効果】
- メイン関数が読みやすくなった
- 描画デモ機能が独立して管理できる
- 新しい描画テストの追加が容易
- コードの責任範囲が明確になった

【draw_test_pattern関数の構成】
1. カラーブロック描画（原色と補色のペア）
2. 線描画パターン（四角形の全接続）
3. フォントテスト（数字とアルファベット）

【次のステップ予想】
- より高度なOS機能の実装
- 複数ファイルへの分割
- モジュール化の進展
- より複雑なシステム管理機能

これでコードの整理が完了し、今後の開発に向けた
清潔で保守しやすい基盤が整いました。

【学習ポイント】
- リファクタリングの重要性
- 関数分離によるコード整理
- 責任の明確化と可読性向上
- 将来の拡張性を考慮したコード構造

p108では技術的な新機能追加はありませんが、
コードの品質向上という重要な改善が行われました。
*/