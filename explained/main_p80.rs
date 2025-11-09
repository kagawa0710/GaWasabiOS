//! WASABI OS - 詳細解説版 main.rs (p80時点)
//! 
//! このファイルは学習用として、p80時点のmain.rsの全ての部分を詳細にコメントで解説しています。
//! 実際のビルドには使用されません。
//!
//! ## 進捗状況
//! - p72まで: 基本的なUEFI起動、画面制御、HLT命令
//! - p80まで: Bitmapトレイト、図形描画関数、効率的な描画システム
//!
//! ## Git履歴での対応
//! - 68cbd50: first commit
//! - 87fdcc2: UEFI graphics output and CPU halt functionality (p72相当)
//! - ae29ca0: detailed code explanation file for learning
//! - 9de5a01: Bitmap trait and pixel-level graphics operations 
//! - 現在: graphics drawing functions and demo (p80相当)

// ============================================================================
// コンパイラ属性：Rustコンパイラに特別な指示を与える
// ============================================================================

#![no_std]   // 標準ライブラリを使わない（OS開発で必須）
#![no_main]  // 通常のmain関数を使わない（UEFIエントリポイント使用）
#![feature(offset_of)] // 構造体オフセット計算の実験的機能を有効化

// ============================================================================
// インポート：必要な機能をcoreライブラリから取り込む
// ============================================================================

use core::mem::offset_of;    // 構造体メンバーのメモリ位置計算
use core::mem::size_of;      // 型のサイズ（バイト数）取得
use core::panic::PanicInfo;  // パニック時の情報
use core::ptr::null_mut;     // NULLポインタ作成
use core::slice;             // メモリスライス操作（現在未使用）
use core::arch::asm;         // インラインアセンブリ（HLT命令用）
use core::cmp::min;          // 最小値計算（境界チェック用）

// ============================================================================
// 型エイリアス：UEFIで使用する型の定義
// ============================================================================

type EfiVoid = u8;    // UEFIの汎用ポインタ型
type EfiHandle = u64; // UEFIオブジェクトの識別子
type Result<T> = core::result::Result<T, &'static str>; // エラーハンドリング型

// ============================================================================
// UEFI GUID：プロトコル識別子
// ============================================================================

#[repr(C)]  // C言語互換メモリレイアウト
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
struct EfiGuid {
    // 128ビットの一意識別子をUEFIプロトコルの区別に使用
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

// ============================================================================
// UEFI ステータスコード
// ============================================================================

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[must_use]  // この値を無視するとコンパイラ警告
#[repr(u64)] // 64ビット整数として表現
enum EfiStatus {
    Success = 0,  // 処理成功（0）、その他はエラー
}

// ============================================================================
// UEFI システムテーブル群：UEFIファームウェアとの接続点
// ============================================================================

#[repr(C)]
struct EfiBootServicesTable {
    // 320バイト目にlocate_protocolが配置されるように調整
    _reserved0: [u64; 40],  // 40 × 8バイト = 320バイト
    
    // プロトコル検索関数：指定したGUIDのプロトコルを取得
    locate_protocol: extern "win64" fn(
        protocol: *const EfiGuid,        // 検索対象のGUID
        registration: *mut EfiVoid,      // 通常NULL
        interface: *mut *mut EfiVoid,    // 結果格納先
    ) -> EfiStatus,
}

// コンパイル時チェック：メモリレイアウトが正しいか確認
const _: () = assert!(offset_of!(EfiBootServicesTable, locate_protocol) == 320);

#[repr(C)]
struct EfiSystemTable {
    _reserved0: [u64; 12],  // 96バイト分の未使用領域
    pub boot_services: &'static EfiBootServicesTable,  // Boot Servicesへの参照
}

const _: () = assert!(offset_of!(EfiSystemTable, boot_services) == 96);

// ============================================================================
// Graphics Output Protocol関連構造体
// ============================================================================

// 画面のピクセル情報
#[repr(C)]
#[derive(Debug)]
struct EfiGraphicsOutputProtocolPixelInfo {
    pub version: u32,               // バージョン情報
    pub horizontal_resolution: u32, // 画面幅（ピクセル）
    pub vertical_resolution: u32,   // 画面高さ（ピクセル）
    _padding0: [u32; 5],           // 未使用領域
    pub pixels_per_scan_line: u32, // 1行あたりのピクセル数
}

const _: () = assert!(size_of::<EfiGraphicsOutputProtocolPixelInfo>() == 36);

// 画面モード情報
#[repr(C)]
#[derive(Debug)]
struct EfiGraphicsOutputProtocolMode<'a> {
    pub max_mode: u32,                                   // 最大モード数
    pub mode: u32,                                       // 現在のモード
    pub info: &'a EfiGraphicsOutputProtocolPixelInfo,  // ピクセル情報
    pub size_of_info: u32,                              // info構造体サイズ
    pub frame_buffer_base: usize,                       // VRAMの開始アドレス
    pub frame_buffer_size: usize,                       // VRAMのサイズ
}

// Graphics Output Protocolメイン構造体
#[repr(C)]
#[derive(Debug)]
struct EfiGraphicsOutputProtocol<'a> {
    reserved: [u64; 3],                                  // 関数ポインタ領域
    pub mode: &'a EfiGraphicsOutputProtocolMode<'a>,   // モード情報
}

// ============================================================================
// プロトコル取得関数
// ============================================================================

fn locate_graphic_protocol<'a>(
    efi_system_table: &'a EfiSystemTable,
) -> Result<&'a EfiGraphicsOutputProtocol<'a>> {
    // Step 1: プロトコル格納用の変数を初期化
    let mut efi_graphics_output_protocol = null_mut::<EfiGraphicsOutputProtocol>();
    
    // Step 2: UEFIのlocate_protocol関数でプロトコルを検索
    let status = (efi_system_table.boot_services.locate_protocol)(
        &EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID,
        null_mut::<EfiVoid>(),
        // 複雑なキャスト：Rustの型安全性とUEFI APIの要求を両立
        &mut efi_graphics_output_protocol as *mut *mut EfiGraphicsOutputProtocol
            as *mut *mut EfiVoid,
    );
    
    // Step 3: エラーチェック
    if status != EfiStatus::Success {
        return Err("Failed to locate graphics outptut protocol");
    }
    
    // Step 4: 安全な参照に変換して返す
    Ok(unsafe { &*efi_graphics_output_protocol })
}

// ============================================================================
// CPU制御関数
// ============================================================================

pub fn hlt() {
    unsafe {
        // x86のHLT命令：CPUを低電力状態にして割り込み待ち
        asm!("hlt");
    }
}

// ============================================================================
// Bitmapトレイト：画像データの抽象化インターフェース（新機能 p80）
// ============================================================================

trait Bitmap {
    // 必須メソッド：実装する構造体が定義する
    fn bytes_per_pixel(&self) -> i64;      // 1ピクセルあたりのバイト数
    fn pixels_per_scan_line(&self) -> i64; // 1行あたりのピクセル数
    fn width(&self) -> i64;                 // 画面幅
    fn height(&self) -> i64;                // 画面高さ
    fn buf_mut(&mut self) -> *mut u8;       // VRAMの生ポインタ

    // デフォルト実装：トレイトが自動提供する便利メソッド

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
            // 境界チェック済みなので安全にunchecked版を呼び出し
            unsafe { Some(&mut *self.unchecked_pixel_at_mut(x, y)) }
        } else {
            None  // 範囲外の場合はNoneを返す
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
// VRAMバッファ情報構造体（新機能 p80）
// ============================================================================

#[derive(Clone, Copy)]
struct VramBefferInfo {
    buf: *mut u8,         // VRAM開始アドレス
    width: i64,           // 画面幅
    height: i64,          // 画面高さ
    pixels_per_line: i64, // 1行あたりのピクセル数（widthと異なる場合あり）
}

// BitmapトレイトをVramBefferInfoに実装
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

// ============================================================================
// VRAM初期化関数
// ============================================================================

fn init_vram(efi_system_table: &EfiSystemTable) -> Result<VramBefferInfo> {
    // Graphics Output Protocolを取得
    let gp = locate_graphic_protocol(efi_system_table)?;

    // UEFI情報からVramBefferInfo構造体を作成
    Ok(VramBefferInfo {
        buf: gp.mode.frame_buffer_base as *mut u8,
        width: gp.mode.info.horizontal_resolution as i64,
        height: gp.mode.info.vertical_resolution as i64,
        pixels_per_line: gp.mode.info.pixels_per_scan_line as i64,
    })
}

// ============================================================================
// 描画関数群（新機能 p80）
// ============================================================================

/// 高速点描画（境界チェックなし）
/// # Safety
/// 座標(x,y)が有効範囲内であることが前提
unsafe fn unchecked_draw_point<T: Bitmap>(buf: &mut T, color: u32, x: i64, y: i64) {
    *buf.unchecked_pixel_at_mut(x, y) = color;
}

/// 安全な点描画（境界チェック付き）
fn draw_point<T: Bitmap>(buf: &mut T, color: u32, x: i64, y: i64) -> Result<()> {
    unsafe {
        // pixel_at_mutが境界チェック済みのポインタを返すので参照外し
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
    
    // 境界チェック済みなので高速なunchecked版を使用
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
// メイン関数：UEFIアプリケーションのエントリポイント
// ============================================================================

#[no_mangle]
fn efi_main(_image_handle: EfiHandle, efi_system_table: &EfiSystemTable) {
    // VRAM初期化
    let mut vram = init_vram(efi_system_table).expect("init_vram failed");
    
    // 画面サイズを取得
    let vw = vram.width;
    let vh = vram.height;
    
    // === 描画デモンストレーション ===
    
    // 1. 背景を黒で塗りつぶし
    fill_rect(&mut vram, 0x000000, 0, 0, vw, vh).expect("fill_rect failed");
    
    // 2. カラフルな矩形を描画
    fill_rect(&mut vram, 0xff0000, 32, 32, 32, 32).expect("fill_rect failed");     // 赤
    fill_rect(&mut vram, 0x00ff00, 64, 64, 64, 64).expect("fill_rect failed");     // 緑
    fill_rect(&mut vram, 0x0000ff, 128, 128, 128, 128).expect("fill_rect failed"); // 青
    
    // 3. グラデーション対角線を描画
    for i in 0..256 {
        let _ = draw_point(&mut vram, 0x010101 * i as u32, i, i);
        // 0x010101 * i = RGB(i,i,i) でグレースケールグラデーション
    }
    
    // 無限ループで画面を保持
    loop {
        hlt()  // 省電力待機
    }
}

// ============================================================================
// パニックハンドラー：エラー発生時の処理
// ============================================================================

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        hlt()  // エラー時も省電力で安全停止
    }
}

// ============================================================================
// 全体アーキテクチャの進化（p72 → p80）
// ============================================================================
/*
【p72時点】
- 基本的なUEFI起動
- 直接VRAMアクセス
- 単純なピクセル操作

【p80時点】
- Bitmapトレイトによる抽象化
- 安全性と効率性を両立した描画システム  
- 再利用可能な図形描画関数

【次のステップ予想】
- フォント描画システム
- ウィンドウ管理
- より複雑なグラフィックス処理

この抽象化により、後の章でより高度な描画機能を効率的に実装できる基盤が完成しました。
*/