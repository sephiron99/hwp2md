# kordoc-bench

hwp2md와 [kordoc](https://github.com/chrisryugj/kordoc)의 HWPX → Markdown 출력을 같은 말뭉치에서 비교한다.
배포하지 않는 내부 도구다(`publish = false`).

## 준비

```bash
# kordoc 빌드 (dist/ 가 있어야 한다)
cd ~/prj/kordoc && npm ci && npm run build
```

## 실행

```bash
cargo run --release -p kordoc-bench -- \
  --corpus "/mnt/c/Users/sephi/Desktop/TMP/hwpx/참고" --sample 200 --seed 1
```

| 옵션 | 기본값 | 뜻 |
|---|---|---|
| `--corpus DIR` | (필수) | HWPX 말뭉치. 하위 디렉터리까지 찾는다 |
| `--sample N` | 100 | 표본 수 (0 = 전부). 같은 seed면 같은 표본 |
| `--seed N` | 1 | 표본 seed |
| `--out DIR` | `target/kordoc-bench` | 결과 디렉터리 |
| `--kordoc DIR` | `~/prj/kordoc` | 빌드된 kordoc 체크아웃 |
| `--node PATH` | `node` | Node.js 실행 파일 |
| `--no-warmup` | | 시간 재기 전 첫 문서로 한 번 예열하지 않음 |
| `--no-stage` | | 표본을 결과 디렉터리로 복사하지 않고 말뭉치에서 바로 변환 |
| `--no-kordoc-scorer` | | kordoc 자체 채점기 기준을 빼고 이 도구 채점만 |

한 문서만 들여다볼 때:

```bash
kordoc-bench --dump-ref 문서.hwpx              # 정답 문단·표 출력
kordoc-bench --dump-ref 문서.hwpx --md 출력.md  # 그 출력을 채점: 못 찾은 문단·긴 잡음
```

## 결과

`<out>/` 아래에:

- `report.md` — 요약 표, kordoc 채점기 기준 표, 지표 설명, 차이가 큰 문서, 실패 목록 (표준 출력에도 찍는다)
- `results.csv` — 문서 × 도구별 수치 (`k_*` 열은 kordoc 채점기)
- `kordoc-score.json` — kordoc 채점기의 요약·문서별 원자료
- `files.tsv` — 번호, 변환한 경로, 말뭉치 안 원래 경로
- `hwp2md/NNNN.md`, `kordoc/NNNN.md` — 두 도구의 출력
- `<도구>/NNNN.missed.txt` — 못 찾은 정답 문단과 정답에 없는 긴 글자열
- `input/NNNN.hwpx` — 표본 복사본 (`--no-stage` 가 아니면)
- `kordoc-stderr.log` — kordoc이 stderr에 낸 것

말뭉치 문서가 결과 디렉터리에 복사되고 출력에 본문이 그대로 들어간다. 기본 위치 `target/` 은 git이 무시한다.

## 측정 방법

**정답**은 두 도구와 코드를 나누지 않는다. HWPX ZIP을 열어 `Contents/sectionN.xml` 의 `<hp:t>` 글을 가장 안쪽
`<hp:p>` 단위로 모으고, 문단이 있는 곳(본문·표 칸·글상자·각주·머리말/꼬리말·숨은 설명)을 붙인다.
채우지 않은 누름틀(`CLICK_HERE`, `dirty≠1`)에 안내 글(`Direction`)이 그대로 들어 있으면 따로 뗀다. 한컴이 인쇄하지 않는 글이다.
안내 글과 다른 값(`0000.00.00.` 처럼 서식에 미리 넣은 값)은 보통 글이다.

**정답 표**는 한글 화면에 보이는 표다(`src/visible.rs`). 한글 문서는 테두리를 감춘 표로 쪽을 짜는 일이 많아
`hp:tbl` 개수가 눈에 보이는 표 개수가 아니다. 그래서 `hp:tbl` 하나를 칸 테두리로 나눈다:

1. 칸 변은 `header.xml` `borderFill` 의 그 변이 `type≠NONE`·색≠`#FFFFFF` 면 보인다. 맞닿은 두 칸 중 한쪽만 그어도 보인다.
2. 위아래로 붙은 짧은 글 두 칸 사이에만 가로선이 있으면 분수로 보고 한 칸으로 합친다.
3. 세로 변이 하나라도 보이는 행, 또는 칸이 둘 이상이고 위아래 가로선이 온 폭에 보이는 행이 "표 행"이다.
   이어진 표 행 묶음 하나가 보이는 표 하나이고, 나머지 행은 글이다. 그 칸 글은 표 칸이 아니라 본문으로 센다.
4. 보이는 표에서 선 밖의 빈 칸, 선 없이 붙은 빈 여백 행, 빈 끝 열을 버리고 어느 칸 모서리도 쓰지 않는 격자선은 접는다.

kordoc `bench/ref/visible-tables.mjs` 와 같은 정의를 코드 공유 없이 따로 구현했다. 표본 200개에서 kordoc 정답과
1,178개 중 1,177개가 행×열까지 같다. 남은 하나는 kordoc 정답이 매핑 안 된 사용자 정의 영역 글자(도장 표시 `󰂙` 등)만
든 칸을 빈 칸으로 쳐서 끝 열을 자르기 때문이다. 한컴은 그 글자를 그리고 kordoc 변환 결과도 그 열을 남기므로 여기서는 내용으로 친다.

**출력**은 두 도구 모두 같은 방식으로 평문으로 바꾼다. 링크 글은 남기고 그림·각주 표지·HTML 태그·엔터티·이스케이프를 걷어 낸다.
파이프 표와 HTML 표는 colspan/rowspan 격자로 행×열을 센다.

| 지표 | 계산 |
|---|---|
| 글 재현율 | 정답 문단 글자(문자·숫자만) 가운데 출력에서 찾은 비율. 문단째 못 찾으면 12글자 조각으로 찾고, 찾은 조각은 앞뒤로 맞는 데까지 늘린다. 6글자 미만 문단은 현재 위치 근처에서만 찾는다 |
| 잡음 | 출력 글자 가운데 어느 정답과도 맞지 않은 비율 |
| 읽기 순서 | 찾은 문단 위치의 최장 증가 부분열 비율 |
| 어절 재현율·정밀도 | 공백으로 나눈 낱말의 다중집합 겹침. 띄어쓰기를 잃으면 깎인다 |
| 표 수·모양 | 2열 이상 보이는 표의 개수와 행×열이 정답과 같은 수 |
| 시간 | 문서마다 읽기+변환+쓰기. hwp2md는 `convert::to_markdown`, kordoc은 한 Node 프로세스 안의 `parse(buf, { filePath })` (CLI 기본 옵션과 같다) |

변환에 실패한 문서는 재현율 0으로 센다. 정답을 읽지 못한 문서(암호화 등)는 품질 지표에서 뺀다.

## kordoc 채점기 기준

kordoc 저장소의 `bench/compare-md-parsers.mjs` 는 다른 파서의 Markdown을 kordoc 자신과 같은 채점기로 잰다.
그 스크립트는 kordoc의 비공개 `bench/corpus` 만 훑으므로 그대로 돌리지 않고 부품을 쓴다:
정답(`bench/ref/hwpx-ref.mjs`)·정렬(`bench/lib/align.mjs`)·표 채점(`bench/lib/table-score.mjs`) 모듈은 그대로 import 하고,
`scoreMd`·`summarize` 함수는 실행할 때마다 스크립트 소스에서 떼어 와 실행한다(`src/kordoc-score.mjs`).
그래서 쓰는 kordoc 체크아웃의 기준과 늘 같다. 함수 이름이 바뀌면 경고를 남기고 이 부분만 건너뛴다.

두 도구 모두 위에서 만든 Markdown을 채점한다. 원 스크립트는 kordoc을 `parse(buf, { filename })` 로 다시 돌리는데,
표본 200개에서 `{ filePath }` 출력과 바이트 단위로 같았다.

이 채점기는 kordoc의 출력 방식과 함께 다듬어졌다. 정답 표 정의(보이는 표)는 이 도구 채점과 같지만,
칸 짜임 대조·글 정렬 같은 세부는 kordoc의 표현 선택과 같을수록 점수가 높으니 이 도구 채점과 함께 본다.

## 한계

- 표 지표는 표현 방식에 따라 달라진다. 틀 표를 통째로 하나의 표로 내는 출력, 바깥 1열 틀 안에 표를 중첩하는 출력은
  보이는 표와 모양이 어긋난다. 절댓값보다 두 도구의 차이를 볼 것.
- 분수 칸 글도 글 재현율에 넣는다(kordoc 정답은 수식으로 따로 센다).
- 문서 하나에서 kordoc이 멈추면 그 뒤 문서는 실패로 남는다. 문서별 시간 제한은 없다.
- 제목 줄 수는 참고값이다. 무엇이 제목인지 정답이 없다.
