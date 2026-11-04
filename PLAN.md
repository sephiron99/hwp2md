# hwp2md — Implementation Plan

HWPX → Markdown 변환기 (Rust). Markdown → HWPX는 deprecated.

## 목표

한글 HWPX 문서를 **LLM이 읽기 좋은** Markdown으로 변환하는 CLI 도구 + 라이브러리.
모든 HWPX 파싱 코드를 자체 구현 — 라이선스 독립성 확보.

> 현재 상태 (2026-11-04)
> - HWP 5.0 바이너리(`.hwp`)는 지원하지 않음 (`20f5d5b`에서 완전 제거). 한컴오피스에서 `.hwpx`로 저장 후 변환.
> - Markdown → HWPX는 deprecated, 개발 중지 (`9e34263`). 한컴오피스에서 열리는 것까지만 확인.
> - 품질은 `tools/kordoc-bench`로 kordoc과 비교해 잰다 (아래 "kordoc-bench" 참고).

## 설계 원칙

1. **라이선스 독립**: HWP 전용 크레이트(unhwp, hwpforge, hwpers 등) 미사용
2. **Generic 의존성만 사용**: zip, quick-xml, comrak 등 범용 크레이트
3. **IR 기반 아키텍처**: 포맷별 reader/writer가 공통 IR을 통해 변환
4. **점진적 구현**: 텍스트 추출 → 서식 → 테이블 → 이미지 → 수식 순

## 의존성

```toml
zip = "2.0"            # ZIP — HWPX 컨테이너
quick-xml = "0.37"     # XML — HWPX 콘텐츠
comrak = "0.34"        # GFM Markdown 파서/렌더러
clap = "4"             # CLI
rayon = "1.10"         # batch 병렬 변환
serde = "1"; serde_yml = "0.0.12"   # 스타일 템플릿(YAML)
thiserror = "2"; anyhow = "1"
tracing = "0.1"; tracing-subscriber = "0.3"
```

HWP 전용이던 `cfb`, `flate2`, `aes`, `byteorder`는 HWP 지원 제거와 함께 뺐다.

## 아키텍처

```
HWPX     ──→ hwpx::reader (ZIP+XML) ──→ table_layout ──→ IR ──→ md::writer ──→ Markdown

Markdown ──→ md::parser (comrak)    ──→ IR ──→ hwpx::writer ──→ HWPX   (deprecated)
```

HWPX 읽기의 핵심 규칙 (2026-10~11 작업):

- **문단 안의 문단**: 각주·표 칸·글상자처럼 문단 안에서 문단이 열리면 바깥 문단을 보관했다가 되살린다.
- **표 안의 표**: 표 상태를 스택으로 쌓는다. 안쪽 표는 그 칸의 블록이 된다.
- **보이는 표만 표로** (`hwpx/table_layout.rs`): `header.xml` `borderFill`로 칸 변이 보이는지 보고, 선이 있는 행 묶음만
  표로 남긴다. 테두리 없는 행은 칸 내용을 읽는 순서대로 문단으로, 1열 표·1×1 상자는 내용만 낸다.
  테두리 정보가 없는 표는 그대로 둔다.
- **누름틀 안내문**: `CLICK_HERE`·`dirty≠1`이고 글이 `Direction` 안내 글과 같으면 내지 않는다(한글이 인쇄하지 않음).
- **각주·미주**: OWPML `<hp:ctrl><hp:footNote>`/`<hp:endNote>`를 읽고, 그 자리에 `[^n]`(미주 `[^eN]`), 정의는 문단 뒤.

### 중간 표현 (IR)

```
Document
├── Metadata (title, author, dates, keywords)
├── Sections[]
│   └── Blocks[]
│       ├── Heading (level, inlines)
│       ├── Paragraph (inlines)
│       ├── Table (rows → cells, col_count)
│       ├── CodeBlock (language, code)
│       ├── BlockQuote (nested blocks)
│       ├── List (ordered, start, items)
│       ├── Image (src, alt)
│       ├── HorizontalRule
│       ├── Footnote (id, content)
│       └── Math (display, tex)
└── Assets[] (name, data, mime_type)
```

### 모듈 구조

```
src/
├── main.rs            — CLI (to-md, to-hwpx(deprecated), info, check, convert, batch)
├── lib.rs / convert.rs / error.rs / ir.rs / style.rs / url_util.rs
├── heading_style.rs   — 제목 스타일·법령 패턴 감지
├── hwpx/
│   ├── reader.rs      — ZIP+XML → IR (header.xml: 글꼴·테두리·메타데이터)
│   ├── handlers.rs    — XML 이벤트 처리 (문단·표·각주·누름틀 …)
│   ├── context/       — 파싱 상태 (문단 스택, 표 스택, 각주, 누름틀 표시)
│   ├── table_layout.rs — 보이는 표 판정·배치
│   └── writer*.rs     — IR → HWPX (deprecated)
└── md/
    ├── writer.rs      — IR → Markdown (표 칸 안 표는 HTML 중첩, 칸 안 줄바꿈 <br>)
    ├── parser.rs      — Markdown → IR (comrak)
    └── html_table.rs  — HTML <table> → IR (중첩 표·<br> 지원)
tools/kordoc-bench/    — kordoc과 출력 비교 벤치 (workspace 멤버, publish=false)
crates/hwp2md-mcp/     — MCP 서버 (별도 바이너리, stdio, rmcp)
├── server.rs          — 도구 4개 (hwpx_to_markdown / hwpx_info / hwpx_tables / list_hwpx), 읽기 전용
├── sandbox.rs         — --root 폴더 밖 경로 거부 (canonicalize, 심볼릭 링크 포함)
├── paging.rs          — 긴 Markdown을 --max-chars 단위로 나눠 offset으로 이어 받기
└── doc.rs             — IR에서 문서 정보·표 데이터 추출
```

## 구현 단계

### Phase 1: 프로젝트 구조 + 기본 파싱 (완료)

자체 구현 기반 프로젝트 재구성.

- [x] 1.1 Cargo.toml — HWP 전용 크레이트 제거, generic 의존성만
- [x] 1.2 IR 설계 — Document/Section/Block/Inline 계층 + Asset
- [x] 1.3 HWP reader — CFB + zlib + record parsing + UTF-16LE text (이후 제거됨)
- [x] 1.4 HWPX reader — ZIP + XML 파싱
- [x] 1.5 MD writer — IR → Markdown (GFM 호환)
- [x] 1.6 MD parser — Markdown → IR (comrak 기반)
- [x] 1.7 HWPX writer — IR → HWPX (ZIP+XML 생성)
- [x] 1.8 Convert 오케스트레이터 — to-md, to-hwpx, info 명령

### Phase 3: HWPX 파싱 고도화 (1주)

HWPX XML 파서 정확도 향상.

- [ ] 3.1 스타일 상속 — header.xml 스타일 정의 파싱
- [ ] 3.2 테이블 — colspan/rowspan 처리
- [ ] 3.3 이미지 — BinData 참조 해결
- [ ] 3.4 각주/미주 — XML 요소 매핑
- [ ] 3.5 수식 — OWPML 수식 요소 → LaTeX
- [ ] 3.6 정부 공문서 HWPX 샘플 검증

### Phase 4: Markdown 렌더러 고도화 (1주)

- [ ] 4.1 GFM 호환 검증 — comrak 왕복 테스트
- [ ] 4.2 복잡한 테이블 — colspan/rowspan → HTML 폴백
- [ ] 4.3 이미지 참조 — 상대경로 / inline base64 옵션
- [ ] 4.4 frontmatter — YAML 메타데이터
- [ ] 4.5 info 명령 — 문서 통계 (페이지, 글자수, 스타일)

### Phase 5: HWPX 라이터 고도화 (1주)

- [ ] 5.1 스타일 매핑 — heading/paragraph 스타일 정의
- [ ] 5.2 테이블 생성 — Markdown 테이블 → HWPX 표
- [ ] 5.3 이미지 삽입 — Asset → HWPX BinData
- [ ] 5.4 YAML 스타일 템플릿 — 커스텀 스타일 지원
- [ ] 5.5 한글 2022+ 호환 검증
- [ ] 5.6 라운드트립 테스트 — HWP→MD→HWPX 왕복 검증

### Phase 6: CLI 완성 + 배포 (1주)

- [ ] 6.1 에러 처리 — 사용자 친화적 에러 메시지
- [ ] 6.2 배치 변환 — 디렉토리 일괄 변환
- [ ] 6.3 CI/CD — GitHub Actions (build + test + clippy)
- [ ] 6.4 테스트 커버리지 80%+
- [ ] 6.5 crates.io 배포
- [ ] 6.6 README 업데이트

## 변환 매핑 테이블

| HWP/HWPX 요소 | Markdown 매핑 | 비고 |
|----------------|---------------|------|
| 제목 (개요 1~6) | `# ~ ######` | 수준 매핑 |
| 본문 | 일반 텍스트 | |
| 굵게/기울임 | `**bold**` / `*italic*` | |
| 밑줄 | `<u>text</u>` | HTML 폴백 |
| 취소선 | `~~text~~` | GFM |
| 위첨자/아래첨자 | `<sup>/<sub>` | HTML 폴백 |
| 하이퍼링크 | `[text](url)` | |
| 표 | GFM table | colspan → HTML 폴백 |
| 이미지 | `![alt](path)` | 파일 추출 |
| 코드 블록 | ` ```lang ``` ` | 고정폭 스타일 추론 |
| 인용 | `> text` | |
| 순서/비순서 목록 | `1.` / `-` | 중첩 지원 |
| 각주 | `[^1]` | GFM footnotes |
| 수식 | `$LaTeX$` | 한글 수식 → LaTeX |
| 머리글/바닥글 | `<!-- header/footer -->` 마커 | 양방향 round-trip |
| 다단 | 단일 단으로 평탄화 | |

## 제한사항

- HWP 5.0 바이너리(`.hwp`)는 입력·출력 모두 지원하지 않음
- Markdown → HWPX는 deprecated — 한컴오피스에서 열리지만 OWPML과 다른 부분이 남음
  (알려진 차이는 `tests/hwpx_validation.rs`의 목록)
- 다단 레이아웃은 단일 단으로 평탄화
- 복잡한 테이블 (colspan/rowspan)·표 안의 표는 HTML 폴백
- 머리말/꼬리말: OWPML `<hp:ctrl><hp:header>`를 머리말로 인식하지 못해 본문 글로 나옴
- 한글 수식은 스크립트를 그대로 출력 (LaTeX 변환 없음)

## Sprint 90 완료 (2026-05-30)

Sprint 89 완료 (2026-05-30). strikeout ~~text~~ + color #span; H1/H2 순서+레벨 이중 핀; MD→HWPX→MD bold/heading 왕복 안정성.

- **P1**: 관(subsection) 감지 — 대형 법령 픽스처 확보 시 검토 (BLOCKER 유지) — 이월
- **P2**: HWPX inline link 통합 테스트 — ✅ multiple links (URL isolation), unsafe URL (writer gate pin)
- **P3**: 복합 인라인 서식 통합 테스트 — ✅ bold+underline → `<u>**text**</u>`, bold+italic+color → `<span>***text***</span>`
- **P4**: 긴 문서 통합 테스트 — ✅ H1/para/H2/code/para IR 위치 순서 + Markdown ATX 순서

## Sprint 91 완료 (2026-05-30)

Sprint 90 완료 (2026-05-30). 1527 tests. hyperlink URL isolation + unsafe gate pin; combined inline formatting; complex doc ordering.

- **P1**: 관(subsection) 감지 — BLOCKER 유지 — 이월
- **P2**: integration.rs 분할 — ✅ `integration_formatting.rs` 분리 (3418 → 3075 lines, -343 lines)
- **P3**: charPr 조합 테스트 — ✅ strikethrough+underline `<u>~~t~~</u>`, bold+strikethrough `~~**t**~~`, italic+underline `<u>*t*</u>`
- **P4**: CTRL_RUBY/BinData — 이미 충분히 커버됨, 스킵

## Sprint 92 완료 (2026-05-30)

Sprint 91 완료 (2026-05-30). 1530 tests. formatting split + 3 combination tests. integration.rs still 3075 lines.

- **P1**: 관(subsection) 감지 — BLOCKER 유지 — 이월
- **P2**: integration_hyperlink.rs 분리 — ✅ 4 hyperlink tests 이전 (3075 → 2913 lines)
- **P3**: 인라인 링크 + 포맷 조합 테스트 — ✅ bold+link `[**t**](url)`, italic+link `[*t*](url)`, color+link `[<span>t</span>](url)`
- **P4**: MD→HWPX→MD 리스트 왕복 — ✅ ordered list IR-structural assertion + text presence + ordering

## Sprint 93 완료 (2026-05-31)

Sprint 92 완료 (2026-05-30). 1534 tests. hyperlink split + formatted link + list roundtrip.

- **P1**: 관(subsection) 감지 — BLOCKER 유지 — 이월
- **P2**: integration_footnote.rs 분리 — ✅ 5 footnote/noteRef tests (2913 → 2719 lines)
- **P3**: 경계 테스트 — ✅ bold+italic+link `[***t***](url)`, paren URL `[t](<url>)` angle-bracket
- **P4**: unordered list roundtrip — ✅ `Block::List { ordered: false }` structural pin

## Sprint 94 완료 (2026-05-31)

Sprint 93 완료 (2026-05-31). 1537 tests. footnote split + paren URL + unordered list roundtrip.

- **P1**: 관(subsection) 감지 — BLOCKER 유지 — 이월
- **P2**: integration_list.rs 분리 — ✅ 5 list tests 이전 (2719 → 2504 lines)
- **P3**: read_fixture → fixtures/mod.rs — ✅ pub fn 공유, 모든 4개 test file에서 local copy 제거
- **P4**: Table MD→HWPX→MD roundtrip — ✅ Block::Table 구조 + 2-column separator + Alice<90<Bob 조합

## Sprint 95 완료 (2026-05-31)

Sprint 94 완료 (2026-05-31). 1538 tests. list split + shared read_fixture + table roundtrip.

- **P1**: 관(subsection) 감지 — BLOCKER 유지 — 이월
- **P2**: integration_blocks.rs 분리 — ✅ 4 block tests 이전 (equation×2, HR, blockquote)
- **P3**: italic/strikethrough roundtrip — ✅ 3-phase 패턴
- **P4**: code block roundtrip — ✅ blockquote/HR 대체 (lossy encoding 발견)

## Sprint 96 완료 (2026-05-31)

Sprint 95 완료 (2026-05-31). 1541 tests. blocks split + italic/strikethrough/code roundtrip.

- **P1**: 관(subsection) 감지 — BLOCKER 유지 — 이월
- **P2**: integration_image.rs 분리 — ✅ 2 image tests 이전 (Sprint 84 P4)
- **P3**: roundtrip helpers — ✅ `ir_hwpx_roundtrip` + `any_inline_in_doc` (top-level paragraphs 한정)
- **P4**: BlockQuote/HR negative assertion — ✅ 손실 계약 pre+!still 패턴으로 executable화

## Sprint 97 완료 (2026-05-31)

Sprint 96 완료 (2026-05-31). 1543 tests. image split + roundtrip helpers + lossy contracts.

- **P1**: 관(subsection) 감지 — BLOCKER 유지 — 이월
- **P2**: integration_table_blocks.rs 분리 — ✅ colspan + no-lang code block 2개 이전
- **P3**: `any_block_in_doc` → fixtures/mod.rs — ✅ lossy-contract tests 리팩 (-12 lines)
- **P4**: 헤딩 roundtrip — ✅ H1/H2/H3 hp:styleIDRef numeric → parse_hwpx_style_ref lossless

## Sprint 98 완료 (2026-06-05) ✅

5인 전문가 병렬 리뷰(Rust 88 / 보안 92 / 포맷 68 / 완성도 62) 후속. CRITICAL 1건 수정으로 S90~S97 8스프린트 연속 production 0건 패딩 종료.

- **P1 (CRITICAL)**: ✅ 이미지 링크 깨짐 수정 — `image_{id}.bin` placeholder → `hwp_to_ir` bin_data 기반 해결(중첩 재귀) + 자산명 단일 출처화. 회귀 테스트 2건(RED→GREEN). 1284 lib tests, clippy clean.
- 이전 로드맵 P2(integration.rs 추가 분할) **폐기** — 파일분할 패딩(`agents.md` Sprint Completion Gate: 동일유형 3회 연속 금지). 측정된 트리거 없이 분할 금지.

## 다음 로드맵 (전문가 리뷰 확정 — 실기능, 패딩 금지)

> 전부 expert-confirmed 실사용 충실도 결함. 착수는 사용자 승인 후.

- **R1 (HIGH)**: ✅ (S99) HWP BinData 압축 해제 — `maybe_decompress_bin` 정책비트×문서플래그, raw 폴백. 결정 매트릭스 5 테스트.
- **R2 (HIGH)**: ✅ (S99) 각주 참조 마커 본문 삽입 — 문단 끝 best-effort `footnote_ref`(위치는 extract가 control char 제거로 복원 불가). ref==def 링크 테스트.
- **R3 (HIGH)**: ✅ (S99) HWP 머리말/꼬리말 읽기 — HeaderFooter variant + head/foot ctrl_id + section hoist. 실 fixture 미검증(스펙 기준), 파싱/hoist 검증.
- **R4 (BLOCKER 종결)**: ✅ (S99) 관(款) 감지 — additive H2 collapse(편/장/절/조 불변→golden 무영향). ~20스프린트 BLOCKER 해소. distinct 5단계는 사람golden 재baseline 필요라 보류.
- **R5**: ✅ (S99) 사람 검증 골든 *준비* — 비자기참조 스캐폴드(`tests/real_human_golden.rs` #[ignore]) + 확인된 실불일치 체크리스트(`HUMAN_VERIFY.md`: 조헤딩 비일관/이중번호119줄/헤딩본문포함). **사람 검증 단계는 사용자 위임**(한컴 렌더 대조 불가).

## 차기 후보 (R5 분석에서 도출, 각 별도 vertical slice — 사용자 승인 후)
- **F-D1/D2**: ✅ (S100) 제N조 헤딩 head/body 분리 — `regulation_heading_split`(ceiling 제거) + `split_inlines_at`. 본문화조 23→0, 헤딩=마커+제목만. 골든 5개 재생성.
- **F-D3**: ✅ (S100) 목록 이중번호 — `strip_leading_list_marker`(ordered+bullet), 골든 5개 재생성(더블마커 0).
- **F-D4**: ✅ (S100) 헤딩 마커 공백 정규화 — generic heading emission 2곳에 `trim_edge_whitespace`(regulation은 F-D1/D2서 해소). 비-regulation 방어.

## Sprint 101 완료 (2026-07-01) ✅

R5/G001·G002 pre-verification에서 로그된 NG3(별표 2 dropped-table + null-equation-byte 버그) 및 목록 번호 연속성 후속. `$<NUL>$` 회귀 + 목록 재번호 수정.

- **NG3**: ✅ HWPTAG_EQEDIT 스크립트 오프셋 버그 수정 — UINT32 attribute word(4바이트)를 건너뛰지 않고 offset 2에서 읽어, 빈 스크립트(len==0)의 attribute 상위 2바이트를 WCHAR 길이 프리픽스로 오독 → 단일 NUL 코드유닛이 `Block::Math { tex: "\0" }`로 leak되어 Markdown에 `$<NUL>$`로 출력되던 결함. `read_utf16le_str(&rec.data, 4)`로 수정 + dispatcher/convert.rs 이중 방어(제어문자·공백뿐인 스크립트는 Math 블록 자체를 생성하지 않음). moel_02_vocational_training 골든 재생성(별표 2 `$$` 라인 제거). 회귀 테스트: `eqedit_real_script_read_at_offset_four`, `eqedit_nul_only_script_is_dropped`, `eqedit_adversarial_*`(5건), `control_to_block_equation_control_only_script_returns_none`, `real_fixtures_no_control_bytes`. 전체 테스트 통과, clippy clean.
- **NG3-list (G002)**: ✅ 목록 번호 연속성 수정 — `group_list_paragraphs`가 모든 `Block::List.start`를 1로 하드코딩 + `strip_leading_list_marker`가 리터럴 서수를 제거해, 1이 아닌 번호로 시작/재개하는 목록(예: 7-1 삽입 후 8·9·10·11, moel_05 서약 2~7)이 1부터 재번호되던 결함. 원본에 HWPTAG_NUMBERING 레코드가 전무(전부 free-typed 마커)함을 raw-record probe로 확인. `leading_ordinal`로 첫 항목 서수를 stripping 전에 포착해 `StagedBlock::ListPara.start` → `Block::List.start`로 전달(writer는 이미 start 존중). 서수 문법 3중복 제거를 위해 `parse_ordinal_marker` 단일화(behavior-preserving: 골든 byte-identical, 85 detect + 42 list 테스트). moel_01/02/03/05 골든 재생성(마커 번호만, 원본 대조). 회귀 테스트: `leading_ordinal_*`, `hwp_to_ir_list_resuming_after_interruption_keeps_source_numbering` 외 adversarial 5건. 1325 lib + 전체 integration 통과, clippy -D warnings clean.
## Sprint 102 완료 (2026-07-11) ✅

사전 크리스탈화된 deep-interview 스펙(`specs/deep-interview-hwp2md-v060-table-format.md`, 4.6% ambiguity) 실행 — v0.6.0 구성 가능한 표 포맷.

- **TableFormat**: ✅ `Html`/`Grid`/`Repeat` enum(`src/convert.rs`) + `ConvertOptions` `#[non_exhaustive]`/`table_format` 빌더 + `--table-format` CLI 플래그(`to-md`, 기본 html). 공개 `to_markdown`/`write_markdown` 시그니처 불변(Html 기본값으로 위임) — 기존 40개+ 호출부 무영향. Grid 모드는 Pandoc 공식 매뉴얼 실 예제로 문법 검증(colspan 경계 병합 + rowspan 연속선분 blank 처리). 적대적 레드팀 8케이스 중 2건에서 rowspan 경계선 렌더링 실버그 발견·수정·영구 회귀 테스트 고정. 9 신규 테스트(유닛 5 + 실 바이너리 subprocess CLI 통합 4). 1330 lib + 27 integration suites 통과, clippy clean, fmt clean.

## Sprint 103 계획 (종결 — 2026-10-26, 아래 "2026-10~11 작업"에서 처리 후 deprecated로 전환)

**착수 배경**: `to-hwpx` 실사용 첫 사례에서 실제 한컴오피스가 산출물을 "손상됨"으로 거부하는 결함 발견(`hwp2md check`는 OK 반환 — 자기참조 검증의 한계가 실사용에서 처음 드러남). Issue [#1](https://github.com/hephaex/hwp2md/issues/1) 등록. R5("사람 검증 골든 준비")가 "사용자 위임"으로 막혀 있던 근본 원인과 동일 갭.

**Domain Charter 확인**: hwp2md는 HWP/HWPX↔Markdown 변환기다. 이 스프린트는 변환 정확성(to-hwpx 산출물의 실제 유효성)에 관한 것으로 core 범위 안.

- **P1 (HIGH)**: 실제 한컴 저작 hwpx 참조 샘플 확보 + 구조적 diff — 공공기관 배포 hwpx 서식 파일 수집(라이선스 문제 없는 공개 배포본), hwp2md 최소 출력물과 `header.xml`(refList 하위요소 순서·필수 속성) / `section0.xml`(`hp:p`/`hp:run` 필수 속성, `hp:linesegarray` 존재 여부) 단위로 diff. 한컴 라이선스 불필요, 가장 저비용.
- **P2 (HIGH)**: P1 diff에서 확인된 구조적 누락/오류 수정 — P1 결과에 따라 구체 항목 결정(사전 확정 안 함, diff 결과가 실제 결함 목록).
- **P3 (MEDIUM)**: OWPML(KS X 6101) XSD 확보 가능 여부 조사 — 가능하면 `to-hwpx` 출력에 스키마 검증 추가(자기참조 파서 재검증이 아닌 독립 스키마 검증). 불가능하면 이 항목은 P4로 강등.
- **P4 (MEDIUM)**: 제3자 hwpx 리더 교차검증 가능성 조사 — `hwpforge`/`hwpConverter` 등 기존 참조 라이브러리로 hwp2md 출력물을 읽어보는 스모크테스트가 CI에 편입 가능한지 검토.
- **P5 (즉시 완화책, LOW 난이도·우선 착수 가능)**: `to-hwpx` 실행 시 CLI stderr 경고 추가("이 출력은 실제 한컴오피스 검증 없이 생성되었습니다, 배포 전 확인 권장") + README 명시. P1~P4 완료 전까지 사용자가 이번처럼 뒤늦게 발견하는 것을 방지하는 최소 조치 — 근본 해결과 별개로 먼저 착수 가능.
- **R (여력 되면, 별도 승인)**: 한컴오피스 체험판 확보해 1회성 수동 검증 파이프라인 구축 — R5를 막던 "한컴 렌더 대조 불가"를 근본적으로 깨는 유일한 방법이나 별도 트랙.

**Demo Gate**: P1~P2는 diff 결과 + 수정 후 재diff(구조 불일치 0건)를 커밋에 포함. P5는 실행 로그(경고 문구 출력 확인)로 증명.

**결과**: 한컴오피스가 멈춘 원인은 패키지 파일(`content.hpf`·`container.xml`·`version.xml`) 형식이었다. 사용자가 진단 파일로
한컴오피스에서 확인했고 `b26df8a`에서 고쳐 열리는 것을 확인했다. 이후 `to-hwpx`는 deprecated로 표시하고 개발을 멈췄다(`9e34263`).
P1·P3는 kordoc 생성기·실제 한글 파일과의 비교와 `polaris_dvc` 스키마 검증으로 대신했다. 남은 OWPML 구조 정리(쪽 설정 위치·표 구조·
`subList` 속성·제목 스타일 속성 이름)는 한컴오피스에서 확인하지 않아 커밋하지 않고 `target/md2hwpx-owpml-wip.patch`에 남겼다
(`target/`은 `cargo clean`으로 지워지므로 필요하면 옮길 것).

## 2026-10~11 작업 (HWPX → Markdown 품질, kordoc 비교)

`d279f82`..`9e34263`, 원격 푸시 완료.

- **CLI**: `batch` 하위 디렉토리 재귀 + 구조 유지, rayon 병렬 변환.
- **HWP 지원 제거** (`20f5d5b`, BREAKING).
- **kordoc-bench** (`tools/kordoc-bench`): 말뭉치 표본을 hwp2md·kordoc으로 변환해 HWPX XML에서 직접 뽑은 정답과 비교.
  이 도구의 채점(글 재현율·잡음·읽기 순서·어절·표 모양)과 kordoc 자체 채점기(`bench/compare-md-parsers.mjs`)를 함께 보고.
  정답 표 = 한글 화면에 보이는 표(kordoc `visible-tables.mjs`와 같은 정의, 따로 구현; 표본 1,178개 중 1,177개 일치).
- **HWPX 읽기 수정**: 각주가 든 문단 글 누락, 표 안의 표 덮어쓰기, 보이는 표만 표로, 누름틀 안내문 빼기 (위 "아키텍처" 참고).
- **Markdown 출력**: 칸 안 표는 HTML 중첩, 칸 안 줄바꿈은 `<br>`.
- **Markdown → HWPX**: 칸 안 표·`<br>` 왕복, 한컴오피스에서 멈추던 패키지 형식, 칸 테두리·주소 수정 후 deprecated.

**kordoc-bench 결과** (말뭉치 3,343개 중 200개, seed 1. "작업 전" = HWP 제거 직후 코드, 같은 채점 기준)

| 지표 | 작업 전 | 현재 | kordoc |
|---|---:|---:|---:|
| 글 재현율 | 95.24% | 99.98% | 99.88% |
| 표 칸 글 재현율 | 89.58% | 99.95% | 99.91% |
| 읽기 순서 | 95.19% | 99.73% | 99.84% |
| 잡음 | 0.83% | 0.08% | 0.18% |
| 표 모양 일치 (정답: 보이는 표 1,178) | 65.87% | 100% | 100% |
| kordoc 채점기 글 누락 0 문서 | 109/200 | 191/200 | 199/200 |
| kordoc 채점기 표 완전 일치 | 59.68% | 99.83% | 99.91% |
| kordoc 채점기 칸 F1 | 0.771 | 0.995 | 0.997 |
| 문서당 변환 시간 (중앙값) | — | 약 0.8 ms | 약 20 ms |

실행: `cargo run --release -p kordoc-bench -- --corpus <말뭉치> --sample 200 --seed 1` (kordoc 체크아웃 `~/prj/kordoc` 빌드 필요).
말뭉치·결과물에는 민감한 문서가 들어 있으므로 `target/` 밖으로 내보내거나 커밋하지 않는다.

## MCP 서버 (2026-11-04)

`crates/hwp2md-mcp`: 별도 바이너리 `hwp2md-mcp`, stdio 통신, 공식 Rust SDK `rmcp` 3.8.
도구 4개 모두 읽기 전용. 본체에는 `convert::read_document`(확장자 확인 + 읽기)만 더함.
테스트: 단위 6 + 실제 바이너리를 stdio로 띄우는 통합 4. 실제 문서 0006(119,147자, 표 72)으로 쪽 나누기·표 확인.
Claude Code·Desktop(로컬 stdio)에 붙음. Claude.ai 웹·ChatGPT는 원격 HTTP만 받으므로 아직 안 됨.

후속 후보: HTTP(Streamable HTTP) 통신 + 경로 대신 파일 내용(base64) 입력 도구 — 원격 클라이언트용.
원격으로 쓰면 문서가 PC 밖으로 나가므로 민감 문서는 사내망 안에서만. 그림 내용 반환(`include_images`)도 후보.

## 다음 후보 (미착수 — 사용자 승인 후)

- **글자 크기 기반 위계**: 앞 문단보다 글자가 작은 문단은 level +1, 같으면 같은 level, 크면 스택에서 되돌아감.
  본문 크기(글자가 가장 많은 크기)보다 큰 문단만 제목으로, 본문보다 작은 문단(※ 참고)은 위 항목에 딸린 문단으로.
  본문 흐름 문단끼리만 비교(빈 문단·표·그림 건너뜀, 표 칸 안 제외), 0.5pt 이내는 같은 크기, 문단 크기는 글자 수 가중 대표값.
  스타일 이름·개요 번호가 있으면 그것이 우선. 표본 확인: 0000(16/14/13/12pt)은 잘 맞고, 0002 개조식(□◦- 모두 15pt)·
  0071 법령(전부 13pt)은 위계를 못 잡음 → 기호·번호 위계는 별도 규칙 필요.
  **전제**: `header.xml` `hh:charPr height`를 `charPrIDRef`로 읽기(지금은 읽지 않음). 검증용 제목 정답 지표도 필요.
- **header 글자 모양 읽기**: 굵게·기울임·밑줄 등을 `charPrIDRef`로 읽기. 색은 `<span style>` 잡음이 되므로 낼지 결정 필요.
- **제목 과다 감지**: 벤치 제목 줄 수 453 대 kordoc 197. 원인 추정(미확인): 숫자 `styleIDRef` 1~6을 제목 수준으로 보는
  규칙이 한글 문서의 스타일 id(1=본문 등)를 제목으로 오인.
- **목록 오인**: `paraPrIDRef ≥ 4`면 목록 항목으로 보는 규칙 때문에 본문 문단에 `  -  `가 붙음.
- **머리말/꼬리말**: OWPML `<hp:ctrl><hp:header>`/`<hp:footer>` 인식 (지금은 `<hp:headerFooter>` 감싸개만 인식).
- **`--help` 영어/한국어**: 실행 시 언어 선택 — `HWP2MD_LANG` > `LC_ALL`/`LC_MESSAGES`/`LANG` > `sys-locale`(Windows),
  영어 doc comment 기본 + 한국어 표로 `mut_subcommand`/`mut_arg` 교체, 누락 검사 테스트. clap 오류 문구는 영어로 남음.
- **남은 kordoc 차이**: kordoc 채점기 글 누락 0 문서 191 대 199 — 원인 미조사.
- 그 밖: 분수 칸 합치기(드묾), 1×1 상자를 인용 블록으로 내는 옵션.

## 라이선스

GPL-3.0-only

## 참조

- [HWP 5.0 스펙](https://www.hancom.com/support/downloadCenter/hwpOwpml) — 한컴 공식
- [OWPML (KS X 6101)](https://tech.hancom.com/hwpxformat/) — HWPX 포맷 설명
- [rhwp](https://github.com/edwardkim/rhwp) — Rust HWP 뷰어/에디터 (참조용)
- [hwpforge](https://docs.rs/hwpforge/) — HWPX 프로그래밍 제어 (참조용)
- [hwpConverter](https://github.com/vsdn/hwpConverter) — Java HWP↔HWPX 변환 (참조용)
