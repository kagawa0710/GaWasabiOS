//! WASABI OS - 詳細解説版 main.rs
//! 
//! このファイルは学習用として、main.rsの全ての部分を詳細にコメントで解説しています。
//! 実際のビルドには使用されません。

// ============================================================================
// コンパイラ属性：Rustコンパイラに特別な指示を与える
// ============================================================================

// #![no_std]: 標準ライブラリを使わない（組み込み・OS開発で必須）
// 標準ライブラリはOSに依存する機能（ファイルシステム、ネットワーク等）を含むため、
// OS自体を作る際は使用できない。代わりにcoreライブラリ（最小限の機能）のみ使用。
#![no_std]

// #![no_main]: 通常のmain関数を使わない
// Rustプログラムは通常main()関数から開始するが、UEFI環境では
// efi_main()という特別な関数がエントリポイントになる
#![no_main]

// #![feature(offset_of)]: 実験的機能を有効化
// offset_of!マクロは構造体のメンバーのメモリ上の位置を取得する
// UEFIの構造体レイアウトが正しいかチェックするために使用
#![feature(offset_of)]

// ============================================================================
// インポート：必要な機能をライブラリから取り込む
// ============================================================================

// core::mem::offset_of: 構造体内のフィールドのオフセット（位置）を計算
// UEFIの構造体がC言語の仕様通りに配置されているか確認するために使用
use core::mem::offset_of;

// core::mem::size_of: 型のサイズ（バイト数）を取得
// VRAMを操作する際にピクセルサイズを計算するために使用
use core::mem::size_of;

// core::panic::PanicInfo: パニック発生時の情報を格納
// Rustでエラーが発生した時の処理を自分で定義するために必要
use core::panic::PanicInfo;

// core::ptr::null_mut: NULLポインタ（何も指さないポインタ）を作成
// UEFIのAPIに「まだ値がセットされていないポインタ」を渡すために使用
use core::ptr::null_mut;

// core::slice: 配列のような連続したメモリ領域を安全に操作
// VRAMを配列として扱うために使用
use core::slice;

// core::arch::asm: インラインアセンブリ（CPUの命令を直接実行）
// HLT命令でCPUを停止させるために使用
use core::arch::asm;

// ============================================================================
// 型エイリアス：既存の型に新しい名前を付ける
// ============================================================================

// EfiVoid: UEFIでの「型が不明な何か」を表現
// C言語のvoid*に相当。u8（1バイト）として扱う
type EfiVoid = u8;

// EfiHandle: UEFIでのハンドル（何かを識別するID）
// 64ビット整数として扱う。UEFIの内部では複雑なオブジェクトの参照
type EfiHandle = u64;

// Result<T>: 成功（Ok）か失敗（Err）かを表現する型
// 通常のstd::result::Resultの代わり。エラー時は文字列メッセージを返す
type Result<T> = core::result::Result<T, &'static str>;

// ============================================================================
// UEFI GUID構造体：UEFIプロトコルの識別子
// ============================================================================

// #[repr(C)]: この構造体をC言語互換のメモリレイアウトにする
// UEFIはC言語で作られているため、同じメモリ配置にする必要がある
#[repr(C)]
// derive: 自動的にトレイト（機能）を実装
// Copy, Clone: データのコピーを許可
// PartialEq, Eq: 等価比較を許可
// Debug: デバッグ出力を許可
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
struct EfiGuid {
    // GUID = Globally Unique Identifier（世界で唯一のID）
    // 128ビット（16バイト）の識別子をUEFIプロトコルの区別に使用
    data0: u32,       // 最初の32ビット
    data1: u16,       // 次の16ビット  
    data2: u16,       // 次の16ビット
    data3: [u8; 8],   // 残りの64ビットを8バイトの配列として
}

// Graphics Output ProtocolのGUID（固定値）
// この番号でUEFIに「画面描画プロトコルを使いたい」と伝える
const EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID: EfiGuid = EfiGuid {
    // 以下の16進数はUEFI仕様書で定められた固定値
    data0: 0x9042a9de,
    data1: 0x23dc,
    data2: 0x4a38,
    data3: [0x96, 0xfb, 0x7a, 0xde, 0xd0, 0x80, 0x51, 0x6a],
};

// ============================================================================
// UEFI ステータス：API呼び出しの結果
// ============================================================================

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
// #[must_use]: この値を使わずに捨てるとコンパイラが警告を出す
// ステータスチェックを忘れるとバグの原因になるため
#[must_use]
// #[repr(u64)]: この列挙型を64ビット整数として表現
#[repr(u64)]
enum EfiStatus {
    // Success = 0: 処理が成功した場合の値
    // UEFIの仕様では0が成功、その他の値はエラーを表す
    Success = 0,
    // 本来はエラーコードも定義するが、今回は成功のみ使用
}

// ============================================================================
// UEFI Boot Services Table：UEFIの基本サービス
// ============================================================================

#[repr(C)]  // C言語互換レイアウト
struct EfiBootServicesTable {
    // _reserved0: 使わないフィールドを40個の64ビット整数で埋める
    // UEFIの仕様書に従い、locate_protocolが320バイト目に配置されるようにする
    _reserved0: [u64; 40],
    
    // locate_protocol: UEFIプロトコルを検索する関数ポインタ
    // extern "win64": Windows x64の呼び出し規約を使用（UEFIの標準）
    locate_protocol: extern "win64" fn(
        protocol: *const EfiGuid,        // 検索したいプロトコルのGUID
        registration: *mut EfiVoid,      // 通常はNULL（使わない）
        interface: *mut *mut EfiVoid,    // 見つかったプロトコルへのポインタを格納
    ) -> EfiStatus,  // 戻り値：成功か失敗か
}

// コンパイル時チェック：locate_protocolが正しい位置（320バイト目）にあるか確認
// もし位置が違うとコンパイルエラーになり、構造体定義のミスに気付ける
const _: () = assert!(offset_of!(EfiBootServicesTable, locate_protocol) == 320);

// ============================================================================
// UEFI System Table：UEFIシステム全体の情報
// ============================================================================

#[repr(C)]
struct EfiSystemTable {
    // 最初の96バイトは今回使わない部分
    _reserved0: [u64; 12],  // 12 × 8バイト = 96バイト
    
    // boot_services: Boot Servicesテーブルへの参照
    // &'static: プログラム実行中ずっと有効な参照
    pub boot_services: &'static EfiBootServicesTable,
}

// コンパイル時チェック：boot_servicesが96バイト目にあるか確認
const _: () = assert!(offset_of!(EfiSystemTable, boot_services) == 96);

// ============================================================================
// Graphics Output Protocol関連の構造体
// ============================================================================

// 画面のピクセル情報を格納する構造体
#[repr(C)]
#[derive(Debug)]
struct EfiGraphicsOutputProtocolPixelInfo {
    pub version: u32,                 // バージョン情報
    pub horizontal_resolution: u32,   // 画面の横幅（ピクセル数）
    pub vertical_resolution: u32,     // 画面の縦幅（ピクセル数）
    _padding0: [u32; 5],             // 未使用領域（20バイト）
    pub pixels_per_scan_line: u32,   // 1行あたりのピクセル数
}

// 構造体サイズが36バイトであることをコンパイル時にチェック
const _: () = assert!(size_of::<EfiGraphicsOutputProtocolPixelInfo>() == 36);

// 画面モードの詳細情報
#[repr(C)]
#[derive(Debug)]
struct EfiGraphicsOutputProtocolMode<'a> {
    pub max_mode: u32,                                      // 利用可能な最大モード数
    pub mode: u32,                                          // 現在のモード番号
    pub info: &'a EfiGraphicsOutputProtocolPixelInfo,     // ピクセル情報への参照
    pub size_of_info: u32,                                 // info構造体のサイズ
    pub frame_buffer_base: usize,                          // VRAM（画面メモリ）の開始アドレス
    pub frame_buffer_size: usize,                          // VRAMのサイズ（バイト数）
}

// Graphics Output Protocolのメイン構造体
#[repr(C)]
#[derive(Debug)]
struct EfiGraphicsOutputProtocol<'a> {
    reserved: [u64; 3],                                    // 未使用（関数ポインタが入る部分）
    pub mode: &'a EfiGraphicsOutputProtocolMode<'a>,     // モード情報への参照
}

// ============================================================================
// Graphics Output Protocolを取得する関数
// ============================================================================

// UEFIシステムからGraphics Output Protocolを検索・取得する関数
fn locate_graphic_protocol<'a>(
    efi_system_table: &'a EfiSystemTable,  // UEFIシステムテーブル
) -> Result<&'a EfiGraphicsOutputProtocol<'a>> {  // 成功時はプロトコルへの参照を返す
    
    // Step 1: プロトコルを格納するためのポインタ変数を作成
    // null_mut(): 「まだ何も指していない」ポインタ
    let mut efi_graphics_output_protocol = null_mut::<EfiGraphicsOutputProtocol>();
    
    // Step 2: UEFIのlocate_protocol関数を呼び出してプロトコルを検索
    let status = (efi_system_table.boot_services.locate_protocol)(
        &EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID,    // 検索したいプロトコルのID
        null_mut::<EfiVoid>(),                 // 登録情報（今回は使わないのでNULL）
        // 複雑なキャスト：Rustの型安全性とUEFI APIの要求を両立させる
        &mut efi_graphics_output_protocol as *mut *mut EfiGraphicsOutputProtocol
            as *mut *mut EfiVoid,
    );
    
    // Step 3: API呼び出しが成功したかチェック
    if status != EfiStatus::Success {
        return Err("Failed to locate graphics outptut protocol");
    }
    
    // Step 4: 成功した場合、ポインタを安全な参照に変換して返す
    // unsafe: 生ポインタを参照に変換する危険な操作
    // UEFIが正しくポインタを設定したと信頼して実行
    Ok(unsafe { &*efi_graphics_output_protocol })
}

// ============================================================================
// CPU制御関数
// ============================================================================

// CPU halt関数：CPUを省電力状態にする
pub fn hlt() {
    unsafe {
        // asm!("hlt"): x86のHLT命令を直接実行
        // HLT = Halt：CPUを停止し、割り込みが来るまで待機
        // 電力消費を抑え、発熱を減らす効果がある
        asm!("hlt");
    }
}

// ============================================================================
// メイン関数：UEFIアプリケーションのエントリポイント
// ============================================================================

// #[no_mangle]: 関数名を変更しない（リンカがefi_mainを見つけられるようにする）
#[no_mangle]
fn efi_main(_image_handle: EfiHandle, efi_system_table: &EfiSystemTable) {
    // このプログラムの目的：画面全体を白色で塗りつぶす
    
    // Step 1: Graphics Output Protocolを取得
    // UEFI環境から画面制御用のプロトコルを検索・取得
    let efi_graphics_output_protocol = locate_graphic_protocol(efi_system_table).unwrap();
    
    // Step 2: VRAM（ビデオメモリ）の情報を取得
    // frame_buffer_base: VRAMが始まるメモリアドレス
    // frame_buffer_size: VRAMのサイズ（バイト数）
    let vram_addr = efi_graphics_output_protocol.mode.frame_buffer_base;
    let vram_byte_size = efi_graphics_output_protocol.mode.frame_buffer_size;
    
    // Step 3: VRAMを安全なRustスライスに変換
    let vram = unsafe {
        // 生のメモリアドレスをu32配列として扱う
        // u32 = 32ビット = 4バイト = 1ピクセル（BGRA形式）
        slice::from_raw_parts_mut(
            vram_addr as *mut u32,                    // VRAMの開始アドレス
            vram_byte_size / size_of::<u32>()        // ピクセル数 = バイト数 ÷ 4
        )
    };
    
    // Step 4: 全ピクセルを白色（0xffffff）で塗りつぶす
    for e in vram {
        // 0xffffff = RGB(255, 255, 255) = 白色
        // BGRAフォーマット：Blue, Green, Red, Alpha の順
        *e = 0xffffff;
    }
    
    // Step 5: 無限ループで画面を保持
    // OSがまだ完成していないので、ここで処理を止めて画面を保つ
    loop {
        hlt()  // CPUを省電力状態にして待機
    }
}

// ============================================================================
// パニックハンドラー：Rustでエラーが発生した時の処理
// ============================================================================

// #[panic_handler]: panic!()が呼ばれた時に実行される関数を指定
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // -> !: この関数は絶対に戻らない（無限ループで止まる）
    
    // エラーが発生したらCPUを停止状態にして安全に停止
    loop {
        hlt()  // 省電力状態で待機を続ける
    }
}

// ============================================================================
// 全体の動作フロー
// ============================================================================
/*
1. UEFIファームウェアがefi_main()を呼び出す
2. Graphics Output Protocolを検索・取得
3. VRAMの位置とサイズを取得
4. VRAMを安全なRustスライスとして扱えるようにする
5. 全ピクセルを白色で塗りつぶす
6. 無限ループで画面を保持（省電力のためHLT命令使用）

エラーが発生した場合：
- パニックハンドラーが無限ループで安全に停止
*/