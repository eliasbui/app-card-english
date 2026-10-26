# Card English

App desktop học từ vựng tiếng Anh cho người Việt, viết bằng Rust, chạy hoàn toàn trên máy, không cần đăng nhập.

## Tính năng

- Lưu từ muốn học, tra phiên âm IPA (CMU Pronouncing Dictionary), nghĩa, giải thích, câu ví dụ bằng AI.
- Nghe phát âm (giọng đọc của hệ điều hành, có chế độ đọc chậm).
- Ghi âm và chấm điểm phát âm bằng Whisper tiny.en chạy trên máy.
- Lịch ôn tập SM-2: từ mới mỗi ngày, ôn đúng lúc sắp quên; nhắc học bằng thông báo hệ thống.
- Bài tập: flashcard, điền từ, chọn đáp án.
- AI gợi ý từ mới theo chủ đề hoặc theo các từ đã học.
- Tạo động lực: XP, cấp độ, mục tiêu mỗi ngày, chuỗi ngày học, biểu đồ 7 ngày, hiệu ứng lật thẻ, pháo giấy, chuỗi trả lời đúng.
- Giao diện sáng, tối hoặc theo hệ thống.
- Kiểm tra phiên bản mới qua GitHub Releases của `eliasbui/app-card-english`.

## AI

Chọn nguồn AI trong Cài đặt. Khi nguồn đang chọn lỗi hoặc hết hạn mức, app tự chuyển sang nguồn khác đã sẵn sàng và ghi rõ ai đã trả lời.

| Nguồn | Cách dùng | Model |
| --- | --- | --- |
| Model trên máy | Tải về trong Cài đặt (Qwen2.5 0.5B, 498 MB), chạy offline | qwen2.5-0.5b-instruct |
| ChatGPT | Bấm Continue with ChatGPT, đăng nhập trong trình duyệt; dùng hạn mức của gói Plus hoặc Pro (gói Free không dùng được) | Lấy từ tài khoản (`GET /v1/models`) |
| ChatGPT qua Codex | Đăng nhập bằng client của Codex CLI; dùng hạn mức Codex của tài khoản, kể cả gói Free khi OpenAI còn tặng. Không chính thức, có thể ngừng chạy bất cứ lúc nào | Lấy từ `backend-api/codex/models` |
| Gemini | API key miễn phí từ Google AI Studio | Lấy từ `models.list`, mặc định là bản flash mới nhất |
| DeepSeek | API key, trả phí theo lượt | Lấy từ `GET /models` |

Chấm phát âm luôn chạy trên máy bằng Whisper tiny.en Q8_0 (44 MB). Trên Mac model chạy bằng GPU (Metal); `CARD_ENGLISH_CPU=1` ép dùng CPU.

ChatGPT dùng luồng chính thức "Sign in with ChatGPT" cho app mã nguồn mở chạy trên máy người dùng
(https://developers.openai.com/siwc/token-sharing-open-source, đang ở giai đoạn preview). App đóng mã hoặc thu phí
phải đăng ký với OpenAI trước. Gemini không có đăng nhập: Google cấm dùng OAuth của Gemini CLI trong phần mềm bên
thứ ba và đã khóa tài khoản vì việc này, nên app dùng API key chính thức.

Đo mức dùng: với từng nguồn, app đếm số lượt, số lỗi và token theo ngày (hôm nay, 7 ngày, 30 ngày, ngày nhiều nhất).
Kèm theo là những gì nhà cung cấp tự báo: số dư DeepSeek, giới hạn Gemini lúc chạm (số lượt mỗi ngày hoặc mỗi phút,
thời gian chờ), header giới hạn của ChatGPT nếu có, và lúc chạm hạn mức. ChatGPT không có API xem hạn mức còn lại;
nút Quản lý hạn mức mở chatgpt.com/settings/usage. Với Codex, app đọc phần trăm đã dùng của hạn mức 5 giờ và hạn mức tuần từ header mỗi câu trả lời, và nút Đo hạn mức gọi `backend-api/wham/usage`.

Key, token và dữ liệu nằm trong `words.db`, quyền đọc chỉ dành cho chủ máy (0600 trên macOS và Linux).

## Dữ liệu

Một file SQLite `words.db` cùng thư mục `models`:

- macOS: `~/Library/Application Support/CardEnglish`
- Windows: `%APPDATA%\CardEnglish`
- Linux: `~/.local/share/card-english`

## Phát triển

```sh
cargo run                         # chạy app
cargo test                        # kiểm thử nhanh
cargo test -- --ignored           # kiểm thử với model thật (cần tải model trước)
cargo run --example make_icons    # tạo lại icon từ assets/icon-source.jpg và tray-source.jpg
scripts/bundle-macos.sh           # tạo dist/Card English.app
```

Cấu trúc:

- `src/main.rs`: cửa sổ (tao + wry), khay hệ thống, lệnh từ giao diện, nhắc nhở, kiểm tra phiên bản.
- `src/db.rs`: từ vựng, cài đặt, lịch SM-2, XP theo ngày.
- `src/ai.rs`: chọn nguồn AI, tự chuyển nguồn khi lỗi, Qwen trên máy.
- `src/chatgpt.rs`: Sign in with ChatGPT (PKCE, làm mới token, đăng xuất), Responses API.
- `src/codex.rs`: ChatGPT qua Codex (đăng nhập, model, hạn mức).
- `src/cloud.rs`: Gemini và DeepSeek.
- `src/speech.rs`: ghi âm (cpal), Whisper, chấm điểm.
- `src/pron.rs`: IPA và so khớp âm vị từ CMUdict.
- `src/update.rs`, `src/models.rs`, `src/net.rs`: phiên bản mới, tải model, HTTP qua curl của hệ thống.
- `ui/index.html`: toàn bộ giao diện.

## Phát hành

```sh
scripts/release.sh 0.2.0
```

Script tăng version, tạo tag và push. GitHub Actions build bản macOS (universal) và Windows x64 rồi đăng lên GitHub Releases; app sẽ tự thấy bản mới.

Lưu ý khi cài: bản macOS chỉ ký ad-hoc, lần đầu mở cần chuột phải chọn Open. Bản Windows cần WebView2 (có sẵn trên Windows 10 và 11).

## Giấy phép dữ liệu

CMU Pronouncing Dictionary: xem `assets/cmudict-LICENSE`. Model Qwen2.5 (Apache 2.0) và Whisper (MIT) được tải trực tiếp từ Hugging Face.
