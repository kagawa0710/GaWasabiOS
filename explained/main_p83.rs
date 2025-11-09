//! WASABI OS - 詳細解説版 main.rs (p83時点)
//! 
//! このファイルは学習用として、p83時点のmain.rsの全ての部分を詳細にコメントで解説しています。
//! 実際のビルドには使用されません。
//!
//! ## 進捗状況
//! - p72まで: 基本的なUEFI起動、画面制御、HLT命令
//! - p80まで: Bitmapトレイト、図形描画関数、効率的な描画システム
//! - p83まで: 線描画アルゴリズム（Bresenham風）、グリッド・放射線描画
//!
//! ## Git履歴での対応
//! - 68cbd50: first commit
//! - 87fdcc2: UEFI graphics output and CPU halt functionality (p72相当)
//! - ae29ca0: detailed code explanation file for learning
//! - 9de5a01: Bitmap trait and pixel-level graphics operations (p80相当)
//! - 現在: line drawing algorithm and grid/radial graphics (p83相当)

// ============================================================================
// コンパイラ属性・インポート（p72からの継続）
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
// 型エイリアス・UEFI関連構造体（p72からの継続）
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

// UEFI ステータスコード
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[must_use]
#[repr(u64)]
enum EfiStatus {
    Success = 0,  // 処理成功（0）、その他はエラー
}

// UEFI Boot Services Table
#[repr(C)]
struct EfiBootServicesTable {
    _reserved0: [u64; 40],  // 320バイト目にlocate_protocolを配置
    locate_protocol: extern "win64" fn(
        protocol: *const EfiGuid,        // 検索対象のGUID
        registration: *mut EfiVoid,      // 通常NULL
        interface: *mut *mut EfiVoid,    // 結果格納先
    ) -> EfiStatus,
}
const _: () = assert!(offset_of!(EfiBootServicesTable, locate_protocol) == 320);

// UEFI System Table
#[repr(C)]
struct EfiSystemTable {
    _reserved0: [u64; 12],
    pub boot_services: &'static EfiBootServicesTable,
}
const _: () = assert!(offset_of!(EfiSystemTable, boot_services) == 96);

// Graphics Output Protocol関連構造体
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
// プロトコル取得・CPU制御関数（p72からの継続）
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
// Bitmapトレイト：画像データの抽象化（p80からの継続）
// ============================================================================

trait Bitmap {
    // 必須メソッド：実装する構造体が定義する
    fn bytes_per_pixel(&self) -> i64;      // 1ピクセルあたりのバイト数
    fn pixels_per_scan_line(&self) -> i64; // 1行あたりのピクセル数
    fn width(&self) -> i64;                 // 画面幅
    fn height(&self) -> i64;                // 画面高さ
    fn buf_mut(&mut self) -> *mut u8;       // VRAMの生ポインタ

    /// 高速ピクセルアクセス（境界チェックなし）
    /// # Safety
    /// 座標(x,y)が有効範囲内であることが前提
    unsafe fn unchecked_pixel_at_mut(&mut self, x: i64, y: i64) -> *mut u32 {
        self.buf_mut()
            .add(((y * self.pixels_per_scan_line() + x) * self.bytes_per_pixel()) as usize)
            as *mut u32
    }

    /// 安全なピクセルアクセス（境界チェック付き）
    fn pixel_at_mut(&mut self, x: i64, y: i64) -> Option<*mut u32> {
        if self.is_in_x_range(x) && self.is_in_y_range(y) {
            unsafe { Some(&mut *self.unchecked_pixel_at_mut(x, y)) }
        } else {
            None
        }
    }

    /// X座標の範囲チェック
    fn is_in_x_range(&self, px: i64) -> bool {
        0 <= px && px < min(self.width(), self.pixels_per_scan_line())
    }

    /// Y座標の範囲チェック
    fn is_in_y_range(&self, py: i64) -> bool {
        0 <= py && py < self.height()
    }
}

// ============================================================================
// VRAMバッファ情報構造体（p80からの継続）
// ============================================================================

#[derive(Clone, Copy)]
struct VramBefferInfo {
    buf: *mut u8,         // VRAM開始アドレス
    width: i64,           // 画面幅
    height: i64,          // 画面高さ
    pixels_per_line: i64, // 1行あたりのピクセル数
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
// 基本描画関数群（p80からの継続）
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
    // 矩形全体が画面内に収まるかチェック
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
// 線描画アルゴリズム（新機能 p83）
// ============================================================================

/// 線形補間による座標計算関数
/// 
/// この関数は線描画において、主軸方向の座標から副軸方向の座標を計算する
/// Bresenham's line algorithmの簡易版
fn calc_slope_point(da: i64, db: i64, ia: i64) -> Option<i64> {
    // da: 主軸方向の距離（長い方）
    // db: 副軸方向の距離（短い方）
    // ia: 主軸方向の現在位置
    // 戻り値: 副軸方向の対応座標
    
    if da < db {
        // 想定外：主軸が副軸より短い（この関数は呼ばれるべきでない）
        None
    } else if da == 0 {
        // 距離0の場合（点）
        Some(0)
    } else if (0..=da).contains(&ia) {
        // 線形補間計算：(2*db*ia + da) / da / 2
        // これにより滑らかな線が描画される
        Some((2 * db * ia + da) / da / 2)
    } else {
        // 範囲外
        None
    }
}

/// 線描画関数（Bresenham風アルゴリズム）
fn draw_line<T: Bitmap>(buf: &mut T, color: u32, x0: i64, y0: i64, x1: i64, y1: i64) -> Result<()> {
    // Step 1: 境界チェック
    if !buf.is_in_x_range(x0)
        || !buf.is_in_y_range(y0)
        || !buf.is_in_x_range(x1)
        || !buf.is_in_y_range(y1)
    {
        return Err("Out of Range");
    }
    
    // Step 2: 線分の属性を計算
    let dx = (x1 - x0).abs();  // X方向の距離
    let dy = (y1 - y0).abs();  // Y方向の距離
    let sx = (x1 - x0).signum();  // X方向の符号（-1, 0, 1）
    let sy = (y1 - y0).signum();  // Y方向の符号（-1, 0, 1）
    
    // Step 3: 主軸を判定して描画
    if dx >= dy {
        // X軸が主軸の場合：横向きに近い線
        for (rx, ry) in (0..dx).flat_map(|rx| calc_slope_point(dx, dy, rx).map(|ry| (rx, ry))) {
            draw_point(buf, color, x0 + rx * sx, y0 + ry * sy)?;
        }
    } else {
        // Y軸が主軸の場合：縦向きに近い線  
        for (rx, ry) in (0..dy).flat_map(|ry| calc_slope_point(dy, dx, ry).map(|rx| (rx, ry))) {
            draw_point(buf, color, x0 + rx * sx, y0 + ry * sy)?;
        }
    }
    Ok(())
}

// ============================================================================
// メイン関数：複雑な描画デモンストレーション（p83の新機能）
// ============================================================================

#[no_mangle]
fn efi_main(_image_handle: EfiHandle, efi_system_table: &EfiSystemTable) {
    // VRAM初期化
    let mut vram = init_vram(efi_system_table).expect("init_vram failed");
    let vw = vram.width;
    let vh = vram.height;
    
    // === 基本描画デモ（p80からの継続） ===
    
    // 背景を黒で塗りつぶし
    fill_rect(&mut vram, 0x000000, 0, 0, vw, vh).expect("fill_rect failed");
    
    // カラフルな矩形
    fill_rect(&mut vram, 0xff0000, 32, 32, 32, 32).expect("fill_rect failed");   // 赤
    fill_rect(&mut vram, 0x00ff00, 64, 64, 64, 64).expect("fill_rect failed");   // 緑
    fill_rect(&mut vram, 0x0000ff, 128, 128, 128, 128).expect("fill_rect failed"); // 青
    
    // グラデーション対角線
    for i in 0..256 {
        let _ = draw_point(&mut vram, 0x010101 * i as u32, i, i);
    }
    
    // === 線描画デモ（p83の新機能） ===
    
    // グリッド描画の設定
    let grid_size: i64 = 32;           // グリッドの間隔（32ピクセル）
    let rect_size: i64 = grid_size * 8;  // 描画領域のサイズ（256ピクセル）
    
    // グリッド線を描画（方眼紙のような見た目）
    for i in (0..=rect_size).step_by(grid_size as usize) {
        // 水平線：左端から右端へ
        let _ = draw_line(&mut vram, 0xff0000, 0, i, rect_size, i);
        
        // 垂直線：上端から下端へ  
        let _ = draw_line(&mut vram, 0xff0000, i, 0, i, rect_size);
    }
    
    // 中心点から放射状の線を描画
    let cx = rect_size / 2;  // 中心X座標（128）
    let cy = rect_size / 2;  // 中心Y座標（128）
    
    for i in (0..=rect_size).step_by(grid_size as usize) {
        // 4方向への放射線
        let _ = draw_line(&mut vram, 0xffff00, cx, cy, 0, i);         // 黄：左向き
        let _ = draw_line(&mut vram, 0x00ffff, cx, cy, i, 0);         // シアン：上向き
        let _ = draw_line(&mut vram, 0xff00ff, cx, cy, rect_size, i); // マゼンタ：右向き
        let _ = draw_line(&mut vram, 0xffffff, cx, cy, i, rect_size); // 白：下向き
    }
    
    // 無限ループで画面を保持
    loop {
        hlt()
    }
}

// ============================================================================
// パニックハンドラー（p72からの継続）
// ============================================================================

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        hlt()
    }
}

// ============================================================================
// アーキテクチャの進化（p72 → p80 → p83）
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

【技術的進歩】
1. **アルゴリズムの実装**: 線形補間による滑らかな線描画
2. **数学的処理**: 座標変換、符号計算、距離計算
3. **パターン描画**: グリッドと放射線による複雑な図形
4. **効率性**: 主軸判定による最適化された描画

【次のステップ予想】
- フォント描画システム
- 円・楕円などの曲線描画
- テクスチャマッピング
- より高速な描画最適化

これで2D描画ライブラリの基礎が完成し、
後の章でのGUIシステム構築の土台が整いました。
*/