# Card English

App desktop học từ vựng cho người Việt, viết bằng Rust, chạy hoàn toàn trên máy, không cần đăng nhập. Hai khóa học: tiếng Anh và tiếng Trung.

## Tính năng

Từ vựng:

- Thêm từ muốn học; AI giải thích từ loại, nghĩa tiếng Việt, cách dùng, câu ví dụ kèm bản dịch, từ đồng nghĩa, trái nghĩa, cụm từ hay đi cùng, trình độ (CEFR A1-C2 hoặc HSK) và chủ đề.
- Tra phiên âm IPA từ CMU Pronouncing Dictionary.
- Lọc danh sách theo trạng thái và theo chủ đề.
- Cửa sổ riêng cho từng từ: luyện phát âm, viết câu, sửa, xóa.

Khóa tiếng Trung:

- Khóa thứ hai, đổi khóa ở thanh bên. Mỗi khóa có lịch ôn riêng và mỗi ngày được học đủ số từ mới đã đặt trong Cài đặt; XP, chuỗi ngày học và thành tích thì dùng chung. Từ có chữ Hán tự vào khóa tiếng Trung, kể cả khi nhập từ file.
- Pinyin có dấu thanh lấy từ CC-CEDICT, âm Hán Việt của từng từ lấy từ tự điển Thiều Chửu, câu ví dụ kèm pinyin, trình độ HSK.
- Nghe đọc bằng giọng tiếng Trung phổ thông của hệ điều hành.
- Chấm phát âm bằng Whisper tiny đa ngôn ngữ (tải thêm 45 MB trong Cài đặt), phân biệt được lỗi sai thanh điệu với lỗi sai âm.
- Bài điền từ và nghe viết nhận cả chữ Hán lẫn pinyin không dấu.
- AI gợi ý từ, chữa câu, viết truyện ngắn và chọn từ trong đoạn văn tiếng Trung dán vào.

Lịch ôn tập:

- FSRS-6 thay cho SM-2; lịch cũ theo SM-2 được chuyển sang tự động.
- Chọn tỉ lệ nhớ mong muốn 80-95%.
- Hoàn tác câu trả lời vừa rồi (phím Z).
- Phím tắt khi ôn: Space, 1-4.

Bài tập:

- Flashcard, điền từ, chọn đáp án, nghe viết (một từ hoặc cả câu ví dụ), nói, viết câu (AI chữa và giải thích bằng tiếng Việt), truyện ngắn (AI viết truyện từ những từ vừa ôn, kèm bản dịch).
- Nguồn từ: tất cả các từ, từ hay quên, từ phát âm còn yếu, hoặc một chủ đề.

Phát âm:

- Ghi âm và chấm điểm một từ hoặc cả câu ví dụ, chỉ ra những từ chưa nghe rõ.
- Chấm điểm bằng Whisper chạy trên máy.

AI gợi ý từ mới: theo chủ đề, theo các từ đã học, hoặc chọn ra từ một đoạn văn dán vào.

Thống kê:

- Tổng XP và cấp độ, chuỗi ngày học hiện tại và dài nhất, độ chính xác 30 ngày, số lượt ôn.
- Lịch học 12 tuần, dự báo lượt ôn 7 ngày tới, danh sách từ hay quên, 12 thành tích.
- Chuỗi ngày học: có thể bật một ngày nghỉ mỗi tuần mà chuỗi vẫn giữ.

Nhắc học:

- Nhắc mỗi N giờ, nhắc vào một giờ cố định mỗi ngày, giờ yên lặng.
- Số từ cần học hiển thị cạnh icon trên thanh menu và trong tooltip của khay hệ thống.
- Khởi động cùng máy tính (LaunchAgent trên macOS, khóa Run trên Windows, mục autostart trên Linux), khi đó app chỉ chạy trong khay.

Giọng đọc: tiếng Anh giọng Mỹ hoặc Anh, chọn giọng cụ thể, chỉnh tốc độ; tiếng Trung dùng giọng phổ thông.

Phím tắt toàn cục (Cmd hoặc Ctrl + Shift + E, và vài tổ hợp khác): thêm từ vừa copy (có tra nghĩa), hoặc nếu là cả đoạn văn thì để AI chọn ra những từ đáng học.

Dữ liệu:

- Xuất CSV (cho Excel, cho Anki) và xuất toàn bộ sang JSON (lịch ôn, XP, lịch sử trả lời; không bao giờ có cài đặt và API key).
- Nhập từ CSV, từ file text xuất ra của Anki (.txt) hoặc từ bản sao JSON; từ đã có trong danh sách thì giữ nguyên.
- Tự sao lưu mỗi ngày vào thư mục `backups` trong thư mục dữ liệu, giữ lại 7 bản gần nhất.

Giao diện: hình dán hoạt hình dễ thương (tạo bằng công cụ sinh ảnh AI), màu nhấn xanh, hồng hoặc tím, giao diện sáng, tối hoặc theo hệ thống.

Cập nhật: app kiểm tra phiên bản mới qua GitHub Releases của `eliasbui/app-card-english`; nếu cài bằng Homebrew hoặc bằng script PowerShell trên Windows thì app hiện lệnh cập nhật thay vì trang tải về.

## AI

Chọn nguồn AI trong Cài đặt. Khi nguồn đang chọn lỗi hoặc hết hạn mức, app tự chuyển sang nguồn khác đã sẵn sàng và ghi rõ ai đã trả lời.

| Nguồn | Cách dùng | Model |
| --- | --- | --- |
| Model trên máy | Tải về trong Cài đặt (Qwen2.5 0.5B, 498 MB), chạy offline | qwen2.5-0.5b-instruct |
| ChatGPT | Bấm Continue with ChatGPT, đăng nhập trong trình duyệt; dùng hạn mức của gói Plus hoặc Pro (gói Free không dùng được) | Lấy từ tài khoản (`GET /v1/models`) |
| ChatGPT qua Codex | Đăng nhập bằng client của Codex CLI; dùng hạn mức Codex của tài khoản, kể cả gói Free khi OpenAI còn tặng. Không chính thức, có thể ngừng chạy bất cứ lúc nào | Lấy từ `backend-api/codex/models` |
| Gemini | API key miễn phí từ Google AI Studio | Lấy từ `models.list`, mặc định là bản flash mới nhất |
| DeepSeek | API key, trả phí theo lượt | Lấy từ `GET /models` |

Chấm phát âm luôn chạy trên máy bằng Whisper: tiếng Anh dùng tiny.en Q8_0 (44 MB), tiếng Trung dùng bản tiny đa ngôn ngữ (tải thêm 45 MB). Trên Mac model chạy bằng GPU (Metal); `CARD_ENGLISH_CPU=1` ép dùng CPU.

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

Một file SQLite `words.db`, thư mục `models` và thư mục `backups`:

- macOS: `~/Library/Application Support/CardEnglish`
- Windows: `%APPDATA%\CardEnglish`
- Linux: `~/.local/share/card-english`

## Cài đặt

Trên macOS, cài bằng Homebrew:

    brew tap eliasbui/card-english
    brew install --cask eliasbui/card-english/card-english

Ghi đủ tên cask như trên thì Homebrew chỉ tin đúng cask này, không cần `brew trust`.

- Cập nhật: `brew upgrade --cask card-english`.
- Gỡ: `brew uninstall --cask card-english`, gỡ luôn mục khởi động cùng máy tính nếu đã bật. Thêm `--zap` để xóa cả dữ liệu.

Trên Windows 10 và 11, mở PowerShell và chạy:

    irm https://raw.githubusercontent.com/eliasbui/app-card-english/main/scripts/install.ps1 | iex

Script cài app cho tài khoản đang dùng vào `%LOCALAPPDATA%\Programs\CardEnglish`, không cần quyền quản trị, kiểm tra file tải về bằng mã SHA-256 mà GitHub công bố, rồi thêm lối tắt vào menu Start.

- Cập nhật: chạy lại lệnh trên. Nếu app đang chạy, script đóng app rồi mở lại sau khi thay.
- Gỡ: Settings > Apps > Card English > Uninstall, gỡ luôn mục khởi động cùng máy tính nếu đã bật. Dữ liệu trong `%APPDATA%\CardEnglish` được giữ lại; xóa thư mục đó nếu muốn xóa cả dữ liệu.

Hoặc tải bản zip trong GitHub Releases và cài bằng tay. Lưu ý:

- macOS: app chỉ ký ad-hoc. Lần đầu mở, macOS sẽ chặn; vào System Settings > Privacy & Security, bấm Open Anyway (hoặc chạy `xattr -dr com.apple.quarantine "/Applications/Card English.app"`).
- Windows: chưa ký số nên SmartScreen có thể cảnh báo, chọn More info > Run anyway. Cần WebView2, có sẵn trên Windows 10 và 11.

## Phát triển

```sh
cargo run                         # chạy app
cargo test                        # kiểm thử nhanh
cargo test -- --ignored           # kiểm thử với model thật (cần tải model trước)
cargo run --example make_icons    # tạo lại icon từ assets/icon-source.jpg và tray-source.jpg
cargo run --example make_stickers # tạo lại ui/stickers.webp (cần cwebp: brew install webp)
scripts/bundle-macos.sh           # tạo dist/Card English.app
```

`make_stickers` đọc các ảnh trong `.pi/generated-images/stickers`, ghi `ui/stickers.webp` và viết lại các rule hình dán trong `ui/index.html`.

Dựng lại hai file dữ liệu tiếng Trung:

```sh
python3 scripts/make-zh-data.py <cedict.txt.gz> <Unihan.zip> <TudienThienChuu.tab> assets
```

Ba file nguồn tải ở:

- https://www.mdbg.net/chinese/export/cedict/cedict_1_0_ts_utf-8_mdbg.txt.gz
- https://www.unicode.org/Public/UCD/latest/ucd/Unihan.zip
- https://raw.githubusercontent.com/catusf/tudien/master/dict/TudienThienChuu.tab

Cấu trúc:

- `src/main.rs`: cửa sổ (tao + wry), khay hệ thống, lệnh từ giao diện, nhắc nhở, kiểm tra phiên bản.
- `src/db.rs`: từ vựng, cài đặt, lịch ôn, XP theo ngày.
- `src/fsrs.rs`: thuật toán FSRS-6 và chuyển lịch SM-2 cũ sang.
- `src/clock.rs`: giờ địa phương cho nhắc học, sao lưu và đếm từ.
- `src/backup.rs`: xuất nhập CSV, Anki, JSON và bản sao mỗi ngày.
- `src/autostart.rs`: khởi động cùng máy tính trên macOS, Windows, Linux.
- `src/ai.rs`: chọn nguồn AI, tự chuyển nguồn khi lỗi, Qwen trên máy.
- `src/chatgpt.rs`: Sign in with ChatGPT (PKCE, làm mới token, đăng xuất), Responses API.
- `src/codex.rs`: ChatGPT qua Codex (đăng nhập, model, hạn mức).
- `src/cloud.rs`: Gemini và DeepSeek.
- `src/speech.rs`: ghi âm (cpal), Whisper, chấm điểm.
- `src/pron.rs`: IPA và so khớp âm vị từ CMUdict, pinyin và âm Hán Việt.
- `src/update.rs`, `src/models.rs`, `src/net.rs`: phiên bản mới, tải model, HTTP qua curl của hệ thống.
- `ui/index.html`: toàn bộ giao diện.
- `ui/stickers.webp`: bộ hình dán.
- `examples/make_stickers.rs`: dựng bộ hình dán từ ảnh gốc.
- `assets/cedict.txt`, `assets/hanviet.txt`: pinyin và âm Hán Việt cho khóa tiếng Trung.
- `scripts/make-zh-data.py`: dựng lại hai file trên từ nguồn.
- `packaging/homebrew/`, `scripts/update-cask.sh`: cask Homebrew và script cập nhật tap.
- `scripts/install.ps1`: cài, cập nhật và đăng ký gỡ app trên Windows.

## Phát hành

```sh
scripts/release.sh 0.2.0
```

Script tăng version, tạo tag và push. GitHub Actions build bản macOS (universal) và Windows x64 rồi đăng lên GitHub Releases; app sẽ tự thấy bản mới.

Sau đó job `homebrew` của workflow trỏ cask trong tap `eliasbui/homebrew-card-english` sang file zip mới, với điều kiện repository secret `HOMEBREW_TAP_TOKEN` (token có quyền push vào tap) đã được đặt. Nếu chưa có token, chạy bằng tay:

```sh
scripts/update-cask.sh 0.2.0
```

Repo tap chỉ cần tạo một lần:

```sh
gh repo create eliasbui/homebrew-card-english --public
```

README của tap là `packaging/homebrew/README.md`, được script chép sang mỗi lần cập nhật.

## Giấy phép dữ liệu

- CMU Pronouncing Dictionary: xem `assets/cmudict-LICENSE`.
- Model Qwen2.5 (Apache 2.0) và Whisper (MIT, cả tiny.en lẫn bản tiny đa ngôn ngữ, từ huggingface.co/lmz/candle-whisper) được tải trực tiếp từ Hugging Face.
- `assets/cedict.txt` lấy từ CC-CEDICT (MDBG và những người đóng góp, bản ngày 2026-10-27, CC BY-SA 4.0), đã bỏ phần định nghĩa tiếng Anh, và bản thân file cũng theo CC BY-SA 4.0. Xem `assets/cedict-LICENSE`.
- `assets/hanviet.txt` chứa âm Hán Việt của Thiều Chửu, Hán Việt tự điển (1942, phạm vi công cộng), theo bản số hóa của catusf/tudien (CC0 1.0), cùng quan hệ dị thể và một phần âm đọc kVietnamese của Unicode Han Database (Unihan), Copyright Unicode, Inc., Unicode License v3. Xem `assets/hanviet-LICENSE`.
- Công thức và tham số mặc định của FSRS-6 theo py-fsrs (open-spaced-repetition, MIT).
- Hình dán: tác giả tự tạo bằng công cụ sinh ảnh AI.
