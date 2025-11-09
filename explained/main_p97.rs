//! WASABI OS - 詳細解説版 main.rs (p97時点)
//! 
//! このファイルは学習用として、p97時点のmain.rsの全ての部分を詳細にコメントで解説しています。
//! 実際のビルドには使用されません。
//!
//! ## 進捗状況
//! - p72まで: 基本的なUEFI起動、画面制御、HLT命令
//! - p80まで: Bitmapトレイト、図形描画関数、効率的な描画システム
//! - p83まで: 線描画アルゴリズム（Bresenham風）、グリッド・放射線描画
//! - p91まで: フォント描画システム、文字レンダリング、テキスト表示
//! - p97まで: 高度なテキスト描画、カーソル機能、改行処理、fmt::Writeトレイト実装
//!
//! ## Git履歴での対応
//! - 68cbd50: first commit
//! - 87fdcc2: UEFI graphics output and CPU halt functionality (p72相当)
//! - ae29ca0: detailed code explanation file for learning
//! - 9de5a01: Bitmap trait and pixel-level graphics operations (p80相当)
//! - 7a25466: line drawing algorithm and complex graphics demo (p83相当)
//! - 1a8562b: font rendering system and bitmap text display (p91相当)
//! - 現在: advanced text rendering with cursor and formatting (p97相当)

// ============================================================================
// コンパイラ属性・インポート（p97での拡張）
// ============================================================================

#![no_std]   // 標準ライブラリを使わない（OS開発で必須）
#![no_main]  // 通常のmain関数を使わない（UEFIエントリポイント使用）
#![feature(offset_of)] // 構造体オフセット計算の実験的機能を有効化

use core::arch::asm;         // インラインアセンブリ（HLT命令用）
use core::cmp::min;          // 最小値計算（境界チェック用）
use core::fmt;               // フォーマット機能（新規追加 p97）
use core::fmt::Write;        // 文字列書き込みトレイト（新規追加 p97）
use core::mem::offset_of;    // 構造体メンバーのメモリ位置計算
use core::mem::size_of;      // 型のサイズ（バイト数）取得
use core::panic::PanicInfo;  // パニック時の情報
use core::ptr::null_mut;     // NULLポインタ作成
use core::writeln;           // 文字列書き込みマクロ（新規追加 p97）

// ============================================================================
// 型エイリアス・UEFI関連構造体（継続）
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

// UEFI関連構造体（詳細は前回と同様）
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[must_use]
#[repr(u64)]
enum EfiStatus {
    Success = 0,
}

#[repr(C)]
struct EfiBootServicesTable {
    _reserved0: [u64; 40],
    locate_protocol: extern "win64" fn(
        protocol: *const EfiGuid,
        registration: *mut EfiVoid,
        interface: *mut *mut EfiVoid,
    ) -> EfiStatus,
}
const _: () = assert!(offset_of!(EfiBootServicesTable, locate_protocol) == 320);

#[repr(C)]
struct EfiSystemTable {
    _reserved0: [u64; 12],
    pub boot_services: &'static EfiBootServicesTable,
}
const _: () = assert!(offset_of!(EfiSystemTable, boot_services) == 96);

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

// ============================================================================
// プロトコル取得・CPU制御関数（継続）
// ============================================================================

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
        return Err("Failed to locate graphics outptut protocol");
    }
    Ok(unsafe { &*efi_graphics_output_protocol })
}

pub fn hlt() {
    unsafe {
        asm!("hlt");  // x86のHLT命令：CPUを低電力状態にして割り込み待ち
    }
}

// ============================================================================
// Bitmapトレイト・VRAM管理（継続）
// ============================================================================

trait Bitmap {
    fn bytes_per_pixel(&self) -> i64;
    fn pixels_per_scan_line(&self) -> i64;
    fn width(&self) -> i64;
    fn height(&self) -> i64;
    fn buf_mut(&mut self) -> *mut u8;

    unsafe fn unchecked_pixel_at_mut(&mut self, x: i64, y: i64) -> *mut u32 {
        self.buf_mut()
            .add(((y * self.pixels_per_scan_line() + x) * self.bytes_per_pixel()) as usize)
            as *mut u32
    }

    fn pixel_at_mut(&mut self, x: i64, y: i64) -> Option<*mut u32> {
        if self.is_in_x_range(x) && self.is_in_y_range(y) {
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
        4  // BGRA = 4バイト/ピクセル
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
    
    if let Ok(target_char) = u8::try_from(c) {
        let mut fi = FONT_SOURCE.split('\n');
        
        while let Some(line) = fi.next() {
            if let Some(line) = line.strip_prefix("0x") {
                if let Ok(idx) = u8::from_str_radix(line, 16) {
                    if idx != target_char {
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
                let pixel_color = match pixel {
                    '*' => color,
                    _ => continue,
                };
                let _ = draw_point(buf, pixel_color, x + dx as i64, y + dy as i64);
            }
        }
    }
}

/// 文字列描画関数（基本版）
/// 
/// 単純な文字列を一列に描画する
fn draw_str_fg<T: Bitmap>(buf: &mut T, x: i64, y: i64, color: u32, s: &str) {
    for (i, c) in s.chars().enumerate() {
        draw_font_fg(buf, x + i as i64 * 8, y, color, c);  // 8ピクセル間隔で配置
    }
}

// ============================================================================
// 高度なテキスト描画システム（新機能 p97）
// ============================================================================

/// VRAM用テキストライター構造体
/// 
/// この構造体は標準的なテキスト書き込みインターフェース（fmt::Write）を
/// VRAM描画に対応させるためのアダプター
struct VramTextWriter<'a> {
    vram: &'a mut VramBefferInfo,    // VRAMへの可変参照
    cursor_x: i64,                   // 現在のカーソルX座標（新機能 p97）
    cursor_y: i64,                   // 現在のカーソルY座標（新機能 p97）
}

/// VramTextWriterの実装
/// 
/// カーソル位置を管理する機能を追加
impl<'a> VramTextWriter<'a> {
    /// コンストラクタ
    /// 
    /// VRAMの参照を受け取り、カーソルを原点(0,0)に初期化
    fn new(vram: &'a mut VramBefferInfo) -> Self {
        Self {
            vram,
            cursor_x: 0,    // カーソルX座標を0で初期化
            cursor_y: 0,    // カーソルY座標を0で初期化
        }
    }
}

/// fmt::Writeトレイトの実装（高度版 p97）
/// 
/// 改行処理とカーソル移動機能を追加した高度なテキスト描画
impl fmt::Write for VramTextWriter<'_> {
    /// 文字列書き込み関数
    /// 
    /// 各文字を順次処理し、改行文字とカーソル移動を適切に処理
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            // 改行文字の処理
            if c == '\n' {
                self.cursor_x = 0;        // カーソルXを行頭に戻す
                self.cursor_y += 16;      // カーソルYを次の行に移動（16ピクセル下）
                continue;
            }
            
            // 通常文字の描画
            draw_font_fg(self.vram, self.cursor_x, self.cursor_y, 0xffffff, c);
            
            // カーソルを次の文字位置に移動
            self.cursor_x += 8;  // 8ピクセル右に移動（文字幅）
        }
        
        Ok(())  // 成功を返す
    }
}

// ============================================================================
// メイン関数：高度なテキスト描画デモ（p97の新機能）
// ============================================================================

#[no_mangle]
fn efi_main(_image_handle: EfiHandle, efi_system_table: &EfiSystemTable) {
    // VRAM初期化
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
    
    // グリッド描画
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
    
    // 個別文字描画
    for (i, c) in "ABCDEF".chars().enumerate() {
        draw_font_fg(&mut vram, i as i64 * 16 + 256, i as i64 * 16, 0xffffff, c);
    }
    
    // 基本文字列描画
    draw_str_fg(&mut vram, 256, 256, 0xffffff, "Hello, world!");
    
    // === 高度なテキスト描画デモ（p97の新機能） ===
    
    // VramTextWriterを作成してformatted出力をテスト
    let mut w = VramTextWriter::new(&mut vram);
    
    // writeln!マクロでformatted出力
    // 各行は自動的に改行され、カーソルが適切に移動する
    for i in 0..4 {
        writeln!(w, "i = {i}").unwrap();  // "i = 0", "i = 1", "i = 2", "i = 3"
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
// アーキテクチャの進化（p72 → p80 → p83 → p91 → p97）
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
- fmt::Writeトレイトによる標準的なテキスト出力インターフェース
- writeln!マクロ対応
- formatted出力機能（変数展開等）

【技術的進歩】
1. **カーソル管理**: テキスト描画位置の自動追跡
2. **改行処理**: \n文字での行送り機能
3. **トレイト実装**: Rustの標準テキスト出力インターフェース対応
4. **マクロ対応**: writeln!等の便利マクロが使用可能
5. **フォーマット機能**: 変数埋め込み（{i}）等の高度な文字列処理

【テキストシステムの仕組み】
- VramTextWriter: VRAMへのテキスト出力を抽象化
- カーソル座標: 現在の文字描画位置を追跡
- fmt::Write実装: Rustの標準的なテキスト出力インターフェース
- 改行処理: カーソル位置の自動調整

【次のステップ予想】
- スクロール機能（画面端での自動改行）
- 文字色・背景色の指定機能
- より高度なテキストレイアウト（タブ、中央寄せ等）
- コンソール入出力システム
- より高度なGUIテキスト表示

これで実用的なテキスト表示システムが完成し、
後の章でのコンソールシステムやGUIアプリケーションの
テキスト表示基盤が整いました。
*/