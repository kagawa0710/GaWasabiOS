//! WASABI OS - 詳細解説版 main.rs (p91時点)
//! 
//! このファイルは学習用として、p91時点のmain.rsの全ての部分を詳細にコメントで解説しています。
//! 実際のビルドには使用されません。
//!
//! ## 進捗状況
//! - p72まで: 基本的なUEFI起動、画面制御、HLT命令
//! - p80まで: Bitmapトレイト、図形描画関数、効率的な描画システム
//! - p83まで: 線描画アルゴリズム（Bresenham風）、グリッド・放射線描画
//! - p91まで: フォント描画システム、文字レンダリング、テキスト表示
//!
//! ## Git履歴での対応
//! - 68cbd50: first commit
//! - 87fdcc2: UEFI graphics output and CPU halt functionality (p72相当)
//! - ae29ca0: detailed code explanation file for learning
//! - 9de5a01: Bitmap trait and pixel-level graphics operations (p80相当)
//! - 7a25466: line drawing algorithm and complex graphics demo (p83相当)
//! - 現在: font rendering system and text display capabilities (p91相当)

// ============================================================================
// コンパイラ属性・インポート（継続）
// ============================================================================

#![no_std]   // 標準ライブラリを使わない（OS開発で必須）
#![no_main]  // 通常のmain関数を使わない（UEFIエントリポイント使用）
#![feature(offset_of)] // 構造体オフセット計算の実験的機能を有効化

use core::mem::offset_of;    // 構造体メンバーのメモリ位置計算
use core::mem::size_of;      // 型のサイズ（バイト数）取得
use core::panic::PanicInfo;  // パニック時の情報
use core::ptr::null_mut;     // NULLポインタ作成
use core::arch::asm;         // インラインアセンブリ（HLT命令用）
use core::cmp::min;          // 最小値計算（境界チェック用）

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
// Bitmapトレイト：画像データの抽象化（継続）
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

// ============================================================================
// VRAMバッファ情報構造体（継続）
// ============================================================================

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

/// 高速点描画（境界チェックなし）
unsafe fn unchecked_draw_point<T: Bitmap>(buf: &mut T, color: u32, x: i64, y: i64) {
    *buf.unchecked_pixel_at_mut(x, y) = color;
}

/// 安全な点描画（境界チェック付き）
fn draw_point<T: Bitmap>(buf: &mut T, color: u32, x: i64, y: i64) -> Result<()> {
    unsafe {
        *(buf.pixel_at_mut(x, y).ok_or("Out of Range")?) = color;
    }
    Ok(())
}

/// 矩形塗りつぶし関数
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
// フォント描画システム（新機能 p91）
// ============================================================================

/// フォントデータ検索関数
/// 
/// この関数は外部ファイル（font.txt）からフォントビットマップを検索・取得する
fn lookup_font(c: char) -> Option<[[char; 8]; 16]> {
    // フォントファイルをコンパイル時に読み込み
    // include_str!マクロにより、font.txtの内容が文字列として埋め込まれる
    const FONT_SOURCE: &str = include_str!("font.txt");
    
    // Step 1: 文字をASCIIコード（u8）に変換を試行
    if let Ok(target_char) = u8::try_from(c) {
        // Step 2: フォントファイルを行ごとに分割して解析
        let mut fi = FONT_SOURCE.split('\n');
        
        while let Some(line) = fi.next() {
            // Step 3: "0x"で始まる行（文字コード定義行）を検索
            if let Some(line) = line.strip_prefix("0x") {
                // Step 4: 16進数文字列をu8に変換
                if let Ok(idx) = u8::from_str_radix(line, 16) {
                    // Step 5: 目的の文字コードと一致するかチェック
                    if idx != target_char {
                        continue;  // 一致しない場合は次の文字へ
                    }
                    
                    // Step 6: 文字が見つかった場合、16行のビットマップを読み込み
                    let mut font = [['*'; 8]; 16];  // 8x16のフォントビットマップ
                    
                    // 次の16行をフォントデータとして取得
                    for (y, line) in fi.clone().take(16).enumerate() {
                        for (x, c) in line.chars().enumerate() {
                            // 配列の範囲チェック付きで文字を設定
                            if let Some(e) = font[y].get_mut(x) {
                                *e = c;
                            }
                        }
                    }
                    
                    return Some(font);  // フォントデータを返す
                }
            }
        }
        
        // 該当する文字が見つからなかった場合
        None
    } else {
        // ASCII変換に失敗した場合（非ASCII文字など）
        None
    }
}

/// フォント前景描画関数
/// 
/// 指定された文字を指定された位置に指定された色で描画する
fn draw_font_fg<T: Bitmap>(buf: &mut T, x: i64, y: i64, color: u32, c: char) {
    // Step 1: 文字のフォントデータを取得
    if let Some(font) = lookup_font(c) {
        // Step 2: フォントビットマップを行ごとに処理
        for (dy, row) in font.iter().enumerate() {
            // Step 3: 行内の各ピクセルを処理
            for (dx, pixel) in row.iter().enumerate() {
                // Step 4: ピクセルの種類に応じて描画
                let pixel_color = match pixel {
                    '*' => color,      // '*'文字の場合は指定色で描画
                    _ => continue,     // その他の文字はスキップ（透明扱い）
                };
                
                // Step 5: 実際のピクセル描画
                // 基準位置(x,y)に相対位置(dx,dy)を加算して描画位置を決定
                let _ = draw_point(buf, pixel_color, x + dx as i64, y + dy as i64);
            }
        }
    }
    // フォントが見つからない場合は何も描画しない
}

// ============================================================================
// メイン関数：フォント描画デモンストレーション（p91の新機能）
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
    fill_rect(&mut vram, 0xff0000, 32, 32, 32, 32).expect("fill_rect failed");   // 赤
    fill_rect(&mut vram, 0x00ff00, 64, 64, 64, 64).expect("fill_rect failed");   // 緑
    fill_rect(&mut vram, 0x0000ff, 128, 128, 128, 128).expect("fill_rect failed"); // 青
    
    // グラデーション対角線
    for i in 0..256 {
        let _ = draw_point(&mut vram, 0x010101 * i as u32, i, i);
    }
    
    // === 線描画デモ（継続） ===
    
    // グリッド描画
    let grid_size: i64 = 32;
    let rect_size: i64 = grid_size * 8;
    
    // グリッド線
    for i in (0..=rect_size).step_by(grid_size as usize) {
        let _ = draw_line(&mut vram, 0xff0000, 0, i, rect_size, i);
        let _ = draw_line(&mut vram, 0xff0000, i, 0, i, rect_size);
    }
    
    // 放射線
    let cx = rect_size / 2;
    let cy = rect_size / 2;
    for i in (0..=rect_size).step_by(grid_size as usize) {
        let _ = draw_line(&mut vram, 0xffff00, cx, cy, 0, i);
        let _ = draw_line(&mut vram, 0x00ffff, cx, cy, i, 0);
        let _ = draw_line(&mut vram, 0xff00ff, cx, cy, rect_size, i);
        let _ = draw_line(&mut vram, 0xffffff, cx, cy, i, rect_size);
    }
    
    // === フォント描画デモ（p91の新機能） ===
    
    // "ABCDEF"の各文字を白色で描画
    // 文字間隔: 16ピクセル（横）
    // 行間隔: 16ピクセル（縦）
    // 開始位置: (256, 0) から斜め下に配置
    for (i, c) in "ABCDEF".chars().enumerate() {
        draw_font_fg(
            &mut vram,
            i as i64 * 16 + 256,  // X座標: 256 + i*16
            i as i64 * 16,        // Y座標: i*16 （斜め配置）
            0xffffff,             // 白色
            c                     // 描画する文字
        );
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
// アーキテクチャの進化（p72 → p80 → p83 → p91）
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

【技術的進歩】
1. **フォントファイル処理**: include_str!による静的ファイル読み込み
2. **文字解析**: ASCII変換、16進数解析、文字列パース
3. **ビットマップフォント**: 8x16ピクセルの文字データ管理
4. **透明度処理**: '*'文字のみ描画し、他は透明扱い
5. **座標変換**: 文字位置から実際のピクセル座標への変換

【フォントシステムの仕組み】
- font.txtファイル: ASCII文字のビットマップデータを格納
- lookup_font(): 指定文字のビットマップを検索・取得
- draw_font_fg(): ビットマップを画面に描画

【次のステップ予想】
- 文字列描画関数（複数文字の連続描画）
- フォントサイズの変更機能
- 背景色付きテキスト描画
- より高度なテキストレイアウト

これでテキスト表示システムの基礎が完成し、
後の章でのコンソール表示やGUI文字表示の土台が整いました。
*/