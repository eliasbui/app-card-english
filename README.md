# Card English

App desktop học từ vựng cho người Việt, viết bằng Rust, chạy hoàn toàn trên máy, không cần đăng nhập. Hai khóa học: tiếng Anh và tiếng Trung.

## Tính năng

Từ vựng:

- Thêm từ muốn học; AI giải thích từ loại, nghĩa tiếng Việt, cách dùng, câu ví dụ kèm bản dịch, từ đồng nghĩa, trái nghĩa, cụm từ hay đi cùng, trình độ (CEFR A1-C2 hoặc HSK) và chủ đề.
- Tra phiên âm IPA từ CMU Pronouncing Dictionary.
- Lọc danh sách theo trạng thái và theo chủ đề.
- Cửa sổ riêng cho từng từ: luyện phát âm, viết câu, sửa, xóa.
- Kiểm tra chính tả ngay khi gõ từ tiếng Anh (theo CMUdict) và gợi ý từ đúng; từ tiếng Trung phải có trong CC-CEDICT.
- Bỏ qua từ không muốn học nữa: từ đó ra khỏi lịch ôn, bài tập và các con số thống kê, bật lại được bất cứ lúc nào.

Khóa tiếng Trung:

- Khóa thứ hai, đổi khóa ở thanh bên. Mỗi khóa là một không gian riêng: lịch ôn, XP, cấp độ, chuỗi ngày học, thành tích, thống kê, màu giao diện, mục tiêu, số từ mới mỗi ngày và giọng đọc đều tách riêng. Nhắc nhở và số trên thanh menu theo khóa đang chọn. Từ có chữ Hán tự vào khóa tiếng Trung, kể cả khi nhập từ file.
- Pinyin có dấu thanh lấy từ CC-CEDICT, âm Hán Việt của từng từ lấy từ tự điển Thiều Chửu, câu ví dụ kèm pinyin, trình độ HSK.
- Nghe đọc bằng giọng Kokoro tiếng Trung (tự nhiên nhất, cho máy mạnh) hoặc Piper Xiao Ya (rất nhẹ, cho máy yếu và trung bình) tải về, chạy offline, hoặc giọng phổ thông của hệ điều hành.
- Chấm phát âm bằng SenseVoice chạy trên máy (tải 166 MB trong Cài đặt), phân biệt được lỗi sai thanh điệu với lỗi sai âm.
- Bài điền từ và nghe viết nhận cả chữ Hán lẫn pinyin không dấu.
- AI gợi ý từ, chữa câu, viết truyện ngắn và chọn từ trong đoạn văn tiếng Trung dán vào.

Lịch ôn tập:

- FSRS-6 thay cho SM-2; lịch cũ theo SM-2 được chuyển sang tự động.
- Chọn tỉ lệ nhớ mong muốn 80-95%.
- Hoàn tác câu trả lời vừa rồi (phím Z).
- Phím tắt khi ôn: Space, 1-4.
- Học theo chủ đề: chọn chủ đề ngay dưới số từ cần ôn ở trang Học hôm nay, hoặc bấm vào tên hay nút Học của một chủ đề trong Từ vựng, Chủ đề; buổi học chỉ gồm từ cần ôn và từ mới của chủ đề đó, vẫn trong giới hạn số từ mới mỗi ngày. Chủ đề chưa có từ thì AI gợi ý từ cho nó, rồi Học chủ đề này thêm các từ và học luôn; hết từ trong buổi học hôm nay thì chuyển sang Học thông minh với các từ của chủ đề.

Bài tập:

- Flashcard, điền từ, chọn đáp án, nghe viết (một từ hoặc cả câu ví dụ), nói, viết câu (AI chữa và giải thích bằng tiếng Việt), truyện ngắn (AI viết truyện từ những từ vừa ôn, kèm bản dịch).
- Nguồn từ: tất cả các từ, từ hay quên, từ phát âm còn yếu, hoặc một chủ đề.

Phát âm:

- Ghi âm và chấm điểm một từ hoặc cả câu ví dụ, chỉ ra những từ chưa nghe rõ.
- Chấm điểm bằng model nhận giọng chạy trên máy (Parakeet).

Gia sư nói (Beta, mục riêng trên thanh bên): trò chuyện 1 kèm 1 bằng giọng nói với gia sư AI, như nói chuyện với một giáo viên thật.

- Bấm Bắt đầu nói rồi cứ nói, ngừng một chút là câu được gửi đi; gia sư trả lời xong thì app nghe tiếp ngay. Muốn chen ngang thì cứ nói, gia sư dừng lại và nghe bạn (đeo tai nghe thì nhạy nhất), hoặc bấm Ngắt. Gõ câu thay cho nói cũng được.
- Gia sư trò chuyện tự nhiên, không bắt nói theo câu mẫu và không bắt bẻ phát âm hay ngữ pháp giữa chừng: câu sai thì gia sư dùng cách nói đúng ngay trong câu đáp rồi nói tiếp. Lỗi được ghi lại lặng lẽ cho phần tổng kết.
- Nói tiếng Việt được: hỏi "câu này nói thế nào", hoặc bí thì nói tiếng Việt, gia sư đáp một câu ngắn bằng tiếng Việt (đọc bằng giọng tiếng Việt) và đưa cách nói bằng ngôn ngữ của khóa.
- Kết thúc và tổng kết: chủ đề, điểm làm tốt, những câu nên nói lại cho đúng, từ và cụm từ đáng nhớ, âm cần luyện, gợi ý cho lần sau, và bài luyện cuối buổi gồm vài câu chuẩn để nghe mẫu và nói thử (có chấm điểm, không bắt buộc).
- Nghe được tiếng Việt, tiếng Anh và câu trộn cả hai bằng Whisper small chạy trên máy (tải 375 MB ngay trên trang, xóa được). Khi có người khác nói xung quanh, app lấy giọng gần micro nhất (người học) và giảm giọng ở xa, coi tiếng nói chuyện nhỏ phía sau là lúc bạn đã ngừng; bỏ các câu Whisper hay tự bịa khi gặp tiếng ồn ("Thank you for watching", "Hãy subscribe kênh...") và nghe lại theo ngôn ngữ của khóa khi nó viết ra chữ của một ngôn ngữ khác. Âm thanh micro được lọc tiếng ù dưới 70 Hz và chống méo khi hạ tần số mẫu, giúp mọi phần chấm phát âm nghe rõ hơn.
- Câu trả lời của ChatGPT (hoặc ChatGPT qua Codex) được đọc từng câu ngay khi viết xong, không suy luận trước, bằng giọng đọc đã chọn trong Cài đặt (chưa tải gói giọng nào thì giọng của máy). Chọn nguồn AI và model ngay trên trang; model nhỏ trả lời nhanh nhất.

AI gợi ý từ mới: theo chủ đề, theo các từ đã học, hoặc chọn ra từ một đoạn văn dán vào.

Thống kê:

- Tổng XP và cấp độ, chuỗi ngày học hiện tại và dài nhất, độ chính xác 30 ngày, số lượt ôn.
- Lịch học 12 tuần, dự báo lượt ôn 7 ngày tới, danh sách từ hay quên, 12 thành tích.
- Chuỗi ngày học: có thể bật một ngày nghỉ mỗi tuần mà chuỗi vẫn giữ.

Nhắc học:

- Nhắc mỗi N giờ, nhắc vào một giờ cố định mỗi ngày, giờ yên lặng.
- Số từ cần học hiển thị cạnh icon trên thanh menu và trong tooltip của khay hệ thống.
- Khởi động cùng máy tính (LaunchAgent trên macOS, khóa Run trên Windows, mục autostart trên Linux), khi đó app chỉ chạy trong khay.

Giọng đọc: các gói giọng chạy trên máy qua sherpa-onnx, tải trong Cài đặt, hoặc giọng của hệ điều hành; chỉnh tốc độ.

| Gói | Dung lượng | Giọng | Hợp với |
| --- | --- | --- | --- |
| Supertonic 3 | 129 MB | 10 giọng nam nữ, đọc cả tiếng Anh và tiếng Việt | Máy yếu và trung bình: nhanh gấp vài lần Kokoro |
| Kokoro | 350 MB | 14 giọng Anh-Mỹ và Anh-Anh tự nhiên nhất | Máy mạnh: đọc một từ dưới một giây |
| Kokoro tiếng Trung | 365 MB | 6 giọng Quan Thoại nam nữ | Máy mạnh |
| Piper Xiao Ya | 14 MB | Một giọng nữ Quan Thoại | Máy yếu và trung bình |

- Cài đặt cho biết cấu hình máy (bộ xử lý, số luồng, RAM, xếp loại yếu, trung bình hay mạnh), gắn nhãn Gợi ý cho máy này lên gói hợp nhất, và ghi thời gian đọc một từ lần gần nhất. Model chạy với số luồng theo máy: khoảng một luồng mỗi nhân, tối đa 4, máy mạnh tối đa 6; trên Mac chỉ tính nhân hiệu năng cao.
- Âm thanh phát ngay trong app qua cpal, không mở trình phát của hệ thống (trên Windows trước đây mỗi lần đọc phải chờ PowerShell khởi động). Đoạn vừa đọc được giữ lại nên bấm nghe lại là phát ngay, giọng đã chọn được mở sẵn khi app khởi động, và trong buổi học từ của hai thẻ kế tiếp được đọc sẵn trong lúc bạn xem thẻ hiện tại.
- Kokoro bản int8 của các phiên bản trước vẫn dùng được nhưng chậm hơn bản mới khoảng 2,5 lần (ONNX Runtime chạy chậm các phép tích chập int8); tải bản mới là bản cũ tự xóa.
- Đọc chuẩn: từ Kokoro không có trong từ điển phát âm (như cellphone, webpage) được đọc thành hai từ ghép lại, số được đọc thành chữ (3, 1,250, 12.5%, 21st, năm 1999), sở hữu cách đọc như số nhiều, giọng Anh-Anh dùng từ điển Anh-Anh; từ vẫn không có cách đọc thì câu đó chuyển sang Supertonic, chưa tải thì sang giọng của hệ điều hành, thay vì bị bỏ mất. Từ đơn được thêm dấu chấm cuối, Kokoro đọc rõ hơn.
- Nút Nghe nghĩa đọc nghĩa tiếng Việt của từ bằng Supertonic, hoặc giọng tiếng Việt của máy nếu có; giọng tiếng Việt chọn riêng trong Cài đặt, dùng chung cho hai khóa.
- Gói giọng đã tải xóa được trong Cài đặt để lấy lại dung lượng; giọng đã chọn từ gói đó quay về giọng của hệ điều hành.

Phím tắt toàn cục (Cmd hoặc Ctrl + Shift + E, và vài tổ hợp khác): thêm từ vừa copy (có tra nghĩa), hoặc nếu là cả đoạn văn thì để AI chọn ra những từ đáng học.

Dữ liệu:

- Xuất CSV (cho Excel, cho Anki) và xuất toàn bộ sang JSON (lịch ôn, XP, lịch sử trả lời; không bao giờ có cài đặt và API key).
- Nhập từ CSV, từ file text xuất ra của Anki (.txt) hoặc từ bản sao JSON; từ đã có trong danh sách thì giữ nguyên.
- Tự sao lưu mỗi ngày vào thư mục `backups` trong thư mục dữ liệu, giữ lại 7 bản gần nhất.

Giao diện: hình dán hoạt hình dễ thương (tạo bằng công cụ sinh ảnh AI), màu nhấn xanh, hồng hoặc tím, giao diện sáng, tối hoặc theo hệ thống.

Cập nhật: app kiểm tra phiên bản mới qua GitHub Releases của `eliasbui/app-card-english` và hỏi trước khi cập nhật. Trên macOS và Windows app tự tải bản mới, kiểm tra kích thước và mã SHA-256 mà GitHub công bố, thay vào chỗ bản đang cài rồi tự mở lại, không cần chạy `brew` hay `irm`. Bản x64 đang chạy giả lập trên máy Windows ARM64 (Snapdragon) được thay bằng bản arm64 khi bản phát hành có. Chỉ khi thư mục cài không ghi được (ví dụ cài cho mọi người dùng) app mới hiện lệnh cập nhật hoặc trang tải về.

Mới trong 0.5.4:

- Trợ lý học làm được việc ngay trong khung chat, gõ tự nhiên có dấu hay không dấu: "Gợi ý 100 từ chủ đề công nghệ" (danh sách từ có phiên âm, nghĩa, nút Nghe, chọn rồi Thêm, AI giải thích từng từ, Gợi ý thêm mỗi lần 10 từ không trùng), "Đố tôi 5 từ hay quên" (bài đố chọn đáp án và điền từ từ chính các từ của bạn, có XP và Vì sao sai?), "Tạo bài tập nối từ chủ đề du lịch", "Thi thử IELTS", "Nhập các từ: a, b, c" hoặc dán một đoạn văn kèm "chọn từ", "Tiến độ của tôi", "Lộ trình của tôi". Các yêu cầu này do app tự nhận ra nên chạy được với mọi model; thẻ trong cuộc trò chuyện được lưu lại.
- Khung chat giống ChatGPT: câu trả lời hiển thị markdown (tiêu đề, danh sách, bảng, trích dẫn), Sao chép mọi tin nhắn, Sửa câu hỏi đã gửi và Hỏi lại câu trả lời cuối, những tin nhắn sau đó được thay bằng câu trả lời mới.
- Trợ lý giúp mọi việc học và dùng ngôn ngữ: dịch câu hay đoạn văn, viết và sửa email, thành ngữ, văn hóa giao tiếp, ôn thi.
- Nút bài tập trong mỗi chặng của lộ trình luyện trên tất cả các chủ đề của chặng, kể cả những từ vừa thêm.

Mới trong 0.5.3:

- Bài thi thử: với lộ trình khó (mục tiêu ghi IELTS, TOEIC, VSTEP, HSK, hoặc trình độ từ B2 trở lên), mỗi chặng kết thúc bằng một bài kiểm tra chặng và chặng cuối là một đề thi thử đủ bốn kỹ năng Nghe, Đọc, Viết, Nói, theo dạng câu hỏi, thứ tự phần thi, thời gian và thang điểm của đề thật (IELTS band 0-9, TOEIC 10-990, VSTEP bậc 3-5, HSK 3.0 kèm HSKK). AI viết đề từ chủ đề của chặng và các từ bạn đã học; đề được lưu lại nên mở lại không phải viết lại. Bài nghe đọc bằng giọng của app đã tải (giọng nam và nữ cho hội thoại), bài nói ghi âm và nhận giọng bằng model trên máy, bài viết và bài nói được AI chấm theo tiêu chí của đề kèm lỗi, cách sửa và bản viết tốt hơn. Chặng chỉ xong khi đủ từ và qua bài kiểm tra, trừ khi bạn tự đánh dấu xong; lộ trình thường vẫn có nút Thi thử. Cần Qwen 1.5B hoặc một nguồn AI trên mạng.
- Trợ lý học: chọn model ngay trong khung chat (áp dụng cho cả Hội thoại nhập vai), tab Lịch sử để mở lại hay xóa các cuộc trò chuyện, nút Cuộc mới, và bấm vào từ tiếng Anh hoặc tiếng Trung in đậm trong câu trả lời để nghe.
- Bấm vào thông báo nhắc học là mở ngay cửa sổ Đố nhanh để ôn các từ đến hạn; chưa có từ nào cần ôn thì app mở cửa sổ chính. Trên macOS thông báo có thêm nút Học ngay, và lời nhắc mới thay cho lời nhắc cũ.
- Khi bạn chưa chọn model trên máy, app tự dùng Qwen 1.5B nếu đã tải, chưa có thì Qwen 0.5B; tải xong Qwen 1.5B là dùng ngay, không cần mở lại app.
- Từ tiếng Trung có nghĩa tiếng Anh ngắn gọn từ CC-CEDICT, có sẵn trong app: hiện trong chế độ Đọc hiểu và được đưa kèm khi AI giải thích từ mới để chọn nghĩa tiếng Việt chính xác hơn.
- Kiểm tra cập nhật qua file `latest.json` của bản phát hành, không còn bị GitHub giới hạn số lần kiểm tra từ một mạng.

Mới trong 0.5.0:

- Lịch ôn tự chia đều: mỗi lần trả lời, từ được hẹn vào ngày ít từ ôn nhất trong vài ngày quanh hạn FSRS. Trong Cài đặt, chọn những ngày bận trong tuần làm ngày ôn nhẹ (mỗi khóa chọn riêng), app sẽ tránh hẹn từ vào những ngày đó. Ở trang Học hôm nay, nút Hoãn từ cần ôn dời các từ đang đến hạn sang 1 đến 7 ngày tới (từ còn nhớ chắc dời xa nhất), nút Ôn trước đưa các từ sắp đến hạn trong 1 đến 3 ngày tới vào buổi học hôm nay.
- Đố nhanh: chọn Đố nhanh trên icon ở khay để mở một cửa sổ nhỏ luôn nằm trên cùng, ôn từng từ đến hạn mà không cần mở app.
- Học thông minh: mỗi từ nhận một dạng bài vừa sức (từ mới chọn đáp án, từ đã quen điền từ và nghe chép, từ đã thuộc luyện nói hoặc đặt câu), các dạng xen kẽ trong một lượt. Trả lời sai thì bấm "Vì sao sai?" để AI giải thích bằng tiếng Việt.
- Trò chơi: Nối từ (nối 6 từ với nghĩa, tính giờ, lưu kỷ lục từng khóa) và Sắp xếp câu (xếp các mảnh thành câu ví dụ rồi xem bản dịch).
- Nói đuổi: app đọc câu ví dụ bằng giọng của app, bạn nói theo ngay; điểm chấm theo từng từ (nói sai từ thì không qua dù đúng ngữ điệu), kèm đường lên xuống giọng của bạn đặt cạnh giọng mẫu (tính theo nửa cung quanh giọng trung bình của mỗi người) và một câu góp ý. Cần một gói giọng của khóa (Kokoro, Supertonic hoặc Piper) để so với giọng mẫu.
- Đọc hiểu (mục Học từ ngữ cảnh trong Bài tập): dán đoạn văn hoặc mở file .txt, .srt, .vtt; mỗi từ được tô màu theo danh sách của bạn (chưa có, đang học, đã thuộc, đã bỏ qua). Bấm vào một từ để nghe, xem phiên âm (tiếng Trung có pinyin và âm Hán Việt) và thêm vào danh sách cùng câu chứa nó.
- Tiện ích trình duyệt cho Chrome, Edge, Brave và Cốc Cốc (thư mục `extension`, cách cài trong `extension/README.md`): bôi đen một từ, bấm chuột phải, chọn "Thêm vào Card English"; từ hiện ra trong app, kể cả khi lúc đó app chưa mở.
- Bộ từ có sẵn: NGSL (2.809 từ tiếng Anh thông dụng nhất), NAWL (từ học thuật), TSL (từ TOEIC), HSK 3.0 cấp 1 đến 7-9. Bấm Thêm 10 / 20 / 50 từ, AI giải thích từng từ trong nền, có thanh tiến độ và nút Dừng. Bài Kiểm tra vốn từ (khoảng 56 câu Biết hoặc Không biết, có vài từ không có thật để trừ phần đoán) ước lượng vốn từ và trình độ, rồi cho các bộ từ bắt đầu ngay sau phần bạn đã biết.
- Chọn nhiều từ: tick ô đầu mỗi dòng (giữ Shift để chọn một dãy) rồi xóa, bỏ qua, học lại hoặc gán chủ đề cho cả nhóm.
- Chủ đề: tự tạo chủ đề với màu và mô tả; đổi tên thì mọi từ đổi theo, xóa chủ đề thì từ vẫn còn.
- Lộ trình: đặt mục tiêu (IELTS, TOEIC, HSK, du lịch...), trình độ, hạn chót hoặc ngày thi và số phút mỗi ngày; AI chia thành 4 đến 12 chặng (chủ đề, số từ, kỹ năng, bài tập nên làm), hoặc tự lập và sửa mọi thứ. App theo dõi tiến độ từng chặng, cho biết bạn đang nhanh hay chậm so với hạn chót, gợi ý số từ mới mỗi ngày, và hiện chặng đang học trên trang chính với nút Học tiếp.
- Trợ lý học: nút tròn ở góc dưới bên phải mở khung trò chuyện với AI ở mọi trang. Trợ lý giúp mọi việc học và dùng ngôn ngữ của khóa đang chọn: giải thích từ, ngữ pháp, thành ngữ, dịch câu hay đoạn văn, viết và sửa email, tin nhắn, văn hóa giao tiếp, ôn thi; đố từ và ôn tập thì dùng chính các từ bạn đã học. Câu hỏi không liên quan tới ngôn ngữ (lập trình, toán, tin tức, sức khỏe, chính trị) được từ chối lịch sự. Lịch sử lưu riêng cho từng khóa và xóa được.
- Hội thoại nhập vai (Bài tập, Học cùng AI): chọn tình huống như phỏng vấn, sân bay, gọi món, khách sạn, hỏi đường, mua sắm, họp; AI đóng vai người đối diện vừa sức bạn, đọc to từng câu, có bản dịch. Trả lời bằng giọng nói (nhận giọng trên máy) hoặc gõ phím; Kết thúc để nhận nhận xét lỗi và cách sửa. Qwen 0.5B còn yếu cho hội thoại, nên dùng Qwen 1.5B hoặc nguồn AI trên mạng.
- Chữ Hán: cửa sổ của mỗi từ tiếng Trung có Thứ tự nét (từng chữ tự viết lại, bộ thủ tô màu riêng) và bộ thủ, thành phần với pinyin và âm Hán Việt (ví dụ 好 = 女 (nữ) + 子 (tử)); mặt sau thẻ cũng có một dòng như vậy. Bài Viết chữ Hán: xem nghĩa và pinyin rồi viết từng nét bằng chuột hoặc cảm ứng. Nét chữ tải một lần cho mỗi chữ rồi dùng được khi không có mạng.
- Luyện thanh điệu (khóa tiếng Trung): nghe một từ và chọn thanh điệu của từng chữ, rồi tự đọc để máy chỉ ra chữ nào sai thanh.
- Hình ảnh cho thẻ từ: dán ảnh, kéo thả hoặc chọn file trong ô sửa từ hay cửa sổ của từ; app tự thu nhỏ và hiện ở mặt sau thẻ, đáp án khi ôn và cửa sổ của từ.
- Đồng bộ Google Drive: Cài đặt, Đăng nhập Google, cho phép trong trình duyệt. Từ vựng, lịch ôn, lịch sử trả lời, XP của từng máy, hình ảnh, chủ đề, lộ trình, lịch sử trò chuyện và các cài đặt học tập được đồng bộ giữa các máy khi mở app, sau mỗi lần học hoặc sửa, mỗi 15 phút và khi bấm Đồng bộ ngay. Dữ liệu nằm trong thư mục ẩn dành riêng cho app trên Google Drive của chính bạn; token đăng nhập không bao giờ nằm trong bản sao lưu.

## Trợ lý chat

Khung chat không chỉ trả lời mà điều khiển được app. Gõ tự nhiên, có dấu hay không dấu:

- Học và điều hướng: "Bắt đầu học", "Mở trang thống kê", "Kế hoạch hôm nay" (các bước kèm nút Bắt đầu), "Thi thử IELTS", "Tạo bài tập nối từ".
- Từ vựng: "Gợi ý 20 từ chủ đề du lịch", "Nhập các từ: a, b, c", "Bỏ qua từ X", "Xóa từ X".
- Cài đặt: "Nhắc tôi học lúc 20:00", "Nhắc tôi mỗi 2 giờ", "Tắt nhắc học", "Đổi sang giao diện tối", "Mỗi ngày học 15 từ mới", "Chuyển sang khóa tiếng Trung".
- Lộ trình: "Tạo lộ trình IELTS 6.5 trong 3 tháng, mỗi ngày 30 phút".
- Hướng dẫn: "Làm sao để thêm từ?", "Đồng bộ ở đâu?", "App này làm được gì?" trả về các bước lấy từ chính các tính năng của app, mỗi bước có nút mở đúng chỗ.

Mọi thao tác ghi, xóa hoặc đổi cài đặt hiện thành thẻ việc cần làm với nút Đồng ý; chỉ khi bạn bấm app mới làm. Thao tác chỉ mở trang hay bắt đầu học thì chạy ngay.

Cách hiểu yêu cầu có hai lớp. Lớp luật của app nhận ra các câu quen thuộc nên chạy được với mọi model, kể cả Qwen trên máy. Với model lớn (ChatGPT, Codex, Gemini, DeepSeek), câu chưa nhận ra được đưa cho một bước lập kế hoạch: AI đề xuất danh sách việc, app kiểm tra từng việc theo danh mục cố định (tên công cụ, giá trị hợp lệ, chỉ các cài đặt học tập) và bỏ việc không hợp lệ. Không có việc nào thì đó là câu hỏi bình thường cho gia sư.

## AI

Chọn nguồn AI trong Cài đặt. Khi nguồn đang chọn lỗi hoặc hết hạn mức, app tự chuyển sang nguồn khác đã sẵn sàng và ghi rõ ai đã trả lời.

| Nguồn | Cách dùng | Model |
| --- | --- | --- |
| Model trên máy | Tải về trong Cài đặt, chạy offline: Qwen2.5 0.5B (498 MB, nhanh) hoặc Qwen2.5 1.5B (1,1 GB, dịch tốt hơn nhưng chậm hơn); chọn bản muốn dùng và xóa bản đã tải để lấy lại chỗ trống | qwen2.5-0.5b-instruct, qwen2.5-1.5b-instruct |
| ChatGPT | Bấm Continue with ChatGPT, đăng nhập trong trình duyệt; dùng hạn mức của gói Plus hoặc Pro (gói Free không dùng được) | Lấy từ tài khoản (`GET /v1/models`) |
| ChatGPT qua Codex | Đăng nhập bằng client của Codex CLI; dùng hạn mức Codex của tài khoản, kể cả gói Free khi OpenAI còn tặng. Không chính thức, có thể ngừng chạy bất cứ lúc nào | Lấy từ `backend-api/codex/models` |
| Gemini | API key miễn phí từ Google AI Studio | Lấy từ `models.list`, mặc định là bản flash mới nhất |
| DeepSeek | API key, trả phí theo lượt | Lấy từ `GET /models` |

Chấm phát âm luôn chạy trên máy, qua sherpa-onnx, bằng model hợp với máy (đo trên Mac M4 với giọng đọc của macOS, từ đơn):

| Model | Dung lượng | Đúng | Một từ | Gợi ý cho |
| --- | --- | --- | --- | --- |
| Parakeet TDT 110M, tiếng Anh | 108 MB | 66,8% | 41 ms | Máy yếu và trung bình |
| Parakeet TDT 0.6B v2, tiếng Anh | 482 MB | 74,5% | 182 ms | Máy mạnh |
| SenseVoice, tiếng Trung | 166 MB | 86,7% | 93 ms | Mọi máy |
| Whisper tiny của các bản trước | 42 MB | 62,9% | 1,2 giây | Chỉ hiện khi đã tải |

Mỗi khóa dùng model bạn chọn (Dùng model này), chưa chọn thì dùng model đã tải hợp với máy nhất; tải về là chọn luôn. Whisper luôn xử lý cả khung 30 giây nên chậm, còn các model mới chỉ nghe đúng đoạn bạn nói. Điểm của từ nói đúng tính theo độ chắc chắn của Parakeet ở từng phần của từ; SenseVoice không cho số này nên từ đúng được 97 điểm. Gia sư nói nghe bằng Whisper small đa ngôn ngữ int8 qua sherpa-onnx (375 MB), tự nhận ra tiếng Việt hay tiếng Anh từng câu: trên Apple M4 mất khoảng 0,5 giây một câu. Trên Mac, Qwen chạy bằng GPU (Metal); `CARD_ENGLISH_CPU=1` ép dùng CPU. Mọi model đã tải đều xóa được trong Cài đặt; hai bản Qwen dùng chung một file tokenizer, file này chỉ bị xóa khi không còn bản nào cần.

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

- Cập nhật: bấm Cập nhật ngay trong app, hoặc `brew upgrade --cask card-english`.
- Gỡ: `brew uninstall --cask card-english`, gỡ luôn mục khởi động cùng máy tính nếu đã bật. Thêm `--zap` để xóa cả dữ liệu.

Trên Windows 10 và 11, mở PowerShell và chạy:

    irm https://raw.githubusercontent.com/eliasbui/app-card-english/main/scripts/install.ps1 | iex

Script cài app cho tài khoản đang dùng vào `%LOCALAPPDATA%\Programs\CardEnglish`, không cần quyền quản trị, kiểm tra file tải về bằng mã SHA-256 mà GitHub công bố, rồi thêm lối tắt vào menu Start. Script tự chọn bản hợp với máy: bản arm64 cho máy Snapdragon (Windows ARM64), bản x64 cho máy Intel và AMD.

- Cập nhật: bấm Cập nhật ngay trong app, hoặc chạy lại lệnh trên. Nếu app đang chạy, script đóng app rồi mở lại sau khi thay.
- Gỡ: Settings > Apps > Card English > Uninstall, gỡ luôn mục khởi động cùng máy tính nếu đã bật. Dữ liệu trong `%APPDATA%\CardEnglish` được giữ lại; xóa thư mục đó nếu muốn xóa cả dữ liệu.

Hoặc tải bản zip trong GitHub Releases và cài bằng tay: `CardEnglish-<version>-macos.zip` cho macOS, `CardEnglish-<version>-windows-arm64.zip` cho Windows trên máy Snapdragon, `CardEnglish-<version>-windows-x64.zip` cho Windows trên máy Intel và AMD. Lưu ý:

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
- `src/speech.rs`: ghi âm (cpal), nhận giọng (Parakeet, SenseVoice, Whisper cũ), chấm điểm.
- `src/talk.rs`: Gia sư nói (Whisper small nghe tiếng Việt và ngôn ngữ của khóa, AI trả lời dạng stream, đọc từng câu).
- `src/pron.rs`: IPA và so khớp âm vị từ CMUdict, pinyin và âm Hán Việt.
- `src/voice.rs`: các gói giọng Kokoro, Supertonic và Piper (tải, xóa, chọn theo máy, chuẩn bị chữ cho từng giọng, giữ đoạn vừa đọc, đọc sẵn, phát qua cpal).
- `src/hardware.rs`: bộ xử lý, RAM, xếp loại máy yếu, trung bình, mạnh và số luồng của model.
- `src/update.rs`, `src/models.rs`, `src/net.rs`: phiên bản mới, tải model, HTTP qua curl của hệ thống.
- `ui/index.html`: toàn bộ giao diện.
- `ui/stickers.webp`: bộ hình dán.
- `examples/make_stickers.rs`: dựng bộ hình dán từ ảnh gốc.
- `assets/cedict.txt`, `assets/hanviet.txt`: pinyin và âm Hán Việt cho khóa tiếng Trung.
- `scripts/make-zh-data.py`: dựng lại hai file trên từ nguồn.
- `packaging/homebrew/`, `scripts/update-cask.sh`: cask Homebrew và script cập nhật tap.
- `scripts/install.ps1`: cài, cập nhật và đăng ký gỡ app trên Windows.
- `scripts/package-windows.sh`: build bản Windows x64 và arm64 ngay trên máy Mac bằng `cargo-xwin` và LLVM của Homebrew.

## Phát hành

```sh
scripts/release.sh 0.2.0
```

Chạy trên máy Mac. Script tăng version, chạy test, build bản macOS universal (Apple silicon và Intel) và hai bản Windows (x64 và arm64) ngay trên máy, tạo tag và push, đăng GitHub Release kèm ba file zip, rồi trỏ cask trong tap `eliasbui/homebrew-card-english` sang file zip macOS; app sẽ tự thấy bản mới. Không dùng GitHub Actions.

Bản Windows được cross-compile sang MSVC ngay trên máy Mac bằng `cargo-xwin` và LLVM của Homebrew (`scripts/package-windows.sh`), vì thư viện sherpa-onnx dựng sẵn cho Windows là bản MSVC. Không cần Docker nữa, chỉ cần cài một lần:

```sh
brew install llvm lld
cargo install --locked cargo-xwin
rustup target add x86_64-pc-windows-msvc aarch64-pc-windows-msvc
```

LLVM của Homebrew chỉ được đưa vào PATH bên trong script, không đụng tới cấu hình shell. Lần build đầu `cargo-xwin` tải MSVC CRT và Windows SDK (khoảng 1 GB, vào `~/Library/Caches/cargo-xwin`), các lần sau dùng lại. Script build cả hai bản; chỉ build một bản thì chạy `ARCH=x64 scripts/package-windows.sh` hoặc `ARCH=arm64 scripts/package-windows.sh`.

Bản arm64 dùng CRT MSVC 14.51 của Visual Studio 2026, tải riêng vào `~/Library/Caches/cargo-xwin-18` (khoảng 1 GB), vì thư viện sherpa-onnx cho ARM64 gọi những hàm của thư viện C++ chuẩn mà CRT 14.44 (bản `cargo-xwin` tải mặc định) không có cho ARM64, nên link báo lỗi thiếu `__std_find_trivial_1` và các hàm cùng loại. Bản x64 vẫn dùng CRT cũ. Test chỉ chạy trên macOS; hai bản Windows được biên dịch và link nhưng không chạy thử, nên sau khi phát hành nên mở thử trên một máy Windows x64 và một máy Snapdragon.

Image và volume Docker của cách build cũ không còn dùng tới; muốn lấy lại chỗ trống thì xóa bằng tay:

```sh
docker image rm cardenglish-win
docker volume rm cardenglish-win-target cardenglish-win-cargo
```

Nếu cask chưa cập nhật được (chưa có quyền push vào tap), chạy lại bằng tay:

```sh
scripts/update-cask.sh 0.2.0
```

Repo tap chỉ cần tạo một lần:

```sh
gh repo create eliasbui/homebrew-card-english --public
```

README của tap là `packaging/homebrew/README.md`, được script chép sang mỗi lần cập nhật.

Đồng bộ Google Drive cần một OAuth client loại Desktop app của Google Cloud (bật Google Drive API, các scope `drive.appdata`, `openid`, `userinfo.email`). Client ID và secret được đưa vào lúc build từ `CARD_ENGLISH_GOOGLE_CLIENT_ID` và `CARD_ENGLISH_GOOGLE_CLIENT_SECRET`; `scripts/release.sh` đọc chúng từ `~/.config/card-english/google-oauth.env` nếu có (không đưa file này vào repo). Thiếu chúng thì bản build vẫn chạy, chỉ không đăng nhập Google được.

## Giấy phép dữ liệu

- CMU Pronouncing Dictionary: xem `assets/cmudict-LICENSE`.
- Giọng Kokoro-82M (Apache 2.0), Supertonic 3 (model OpenRAIL-M, mã MIT, Supertone Inc.) và Piper zh_CN xiao_ya (dữ liệu Biaobei, chỉ dùng phi thương mại) đóng gói bởi sherpa-onnx (Apache 2.0), tải từ github.com/k2-fsa/sherpa-onnx/releases, kiểm tra bằng SHA-256.
- Model nhận giọng Parakeet TDT 110M và 0.6B v2 (CC-BY-4.0, NVIDIA) và SenseVoice Small (giấy phép model của FunASR) đóng gói bởi sherpa-onnx, tải từ github.com/k2-fsa/sherpa-onnx/releases, kiểm tra bằng SHA-256.
- Model Qwen2.5 (Apache 2.0) và Whisper (MIT, cả tiny.en lẫn bản tiny đa ngôn ngữ, từ huggingface.co/lmz/candle-whisper) được tải trực tiếp từ Hugging Face.
- `assets/cedict.txt` và `assets/cedict-gloss.txt.gz` lấy từ CC-CEDICT (MDBG và những người đóng góp, bản ngày 2026-10-27, CC BY-SA 4.0): file thứ nhất bỏ phần định nghĩa tiếng Anh, file thứ hai giữ nghĩa tiếng Anh rút gọn của từng mục từ; cả hai cũng theo CC BY-SA 4.0. Xem `assets/cedict-LICENSE`.
- `assets/hanviet.txt` chứa âm Hán Việt của Thiều Chửu, Hán Việt tự điển (1942, phạm vi công cộng), theo bản số hóa của catusf/tudien (CC0 1.0), cùng quan hệ dị thể và một phần âm đọc kVietnamese của Unicode Han Database (Unihan), Copyright Unicode, Inc., Unicode License v3. Xem `assets/hanviet-LICENSE`.
- Công thức và tham số mặc định của FSRS-6 theo py-fsrs (open-spaced-repetition, MIT).
- Hình dán: tác giả tự tạo bằng công cụ sinh ảnh AI.
- `assets/ngsl.txt`, `assets/nawl.txt`, `assets/tsl.txt`: New General Service List và New Academic Word List của Browne, C., Culligan, B. và Phillips, J., TOEIC Service List của Browne, C. và Culligan, B. (bản 1.2, newgeneralservicelist.com, CC BY-SA 4.0), chỉ giữ từ gốc theo thứ tự tần suất; các file này cũng theo CC BY-SA 4.0. Xem `assets/ngsl-LICENSE`.
- `assets/hsk30.txt`: danh sách từ và cấp độ HSK 3.0 (GF 0025-2021) lấy từ ivankra/hsk30 (MIT). Xem `assets/hsk30-LICENSE`.
- `assets/hanzi-parts.txt`: bộ thủ và thành phần chữ lấy từ dictionary.txt của Make Me a Hanzi (LGPL 3 trở lên). Xem `assets/hanzi-parts-LICENSE`.
- Hanzi Writer (MIT) trong `ui/parts/hanzi-0-writer.js`, và dữ liệu nét chữ hanzi-writer-data từ Make Me a Hanzi (Arphic Public License), tải khi cần. Xem `assets/hanzi-writer-LICENSE`.
