# How Excel measures a string, settled by two independent routes.
#
# The first capture found LEN(UNICHAR(128512)) = 1, not 2. That contradicts the
# usual assumption that Excel is UTF-16 throughout and it happens to agree with
# Rust's chars().count(), which is what usk-formula already does -- so it matters
# a great deal whether the finding is real or an artefact of UNICHAR.
#
# So every case here runs twice over the same characters: once built by UNICHAR()
# inside a formula, and once seeded into a cell as a real .NET string through the
# `codepoints` fixture field, which is a different path into Excel entirely. If
# the two disagree, the divergence is in UNICHAR and not in LEN. If they agree,
# the engine's existing char-based offsets are correct and no change is needed.
#
# Whichever way it lands, every string offset in LEFT / RIGHT / MID / FIND /
# SEARCH / REPLACE inherits the answer, so this is the highest-leverage question
# in the text grid.
@{
    schema = 'ehkatra.oracle.grid/1'
    group  = 'text'

    functions = @(
        @{
            name = '__compat_string_length'
            doc  = 'Code units versus Unicode scalars, cross-checked between a UNICHAR formula and a cell seeded with the same code points.'
            fixture = @(
                @{ ref = 'A1'; codepoints = @(128512) }                  # emoji, astral
                @{ ref = 'A2'; codepoints = @(128512, 120) }             # emoji + x
                @{ ref = 'A3'; codepoints = @(233) }                     # e-acute, precomposed
                @{ ref = 'A4'; codepoints = @(101, 769) }                # e + combining acute
                @{ ref = 'A5'; codepoints = @(20320, 22909) }            # two CJK
                @{ ref = 'B1'; codepoints = @(160) }                     # non-breaking space
                @{ ref = 'B2'; codepoints = @(160, 97, 98, 99, 160) }    # NBSP abc NBSP
                @{ ref = 'B3'; codepoints = @(55357, 56832) }            # the surrogate pair of U+1F600, written directly
                @{ ref = 'B4'; codepoints = @(128512, 128513, 128514) }  # three astral characters
                @{ ref = 'B5'; codepoints = @(223) }                     # sharp s
            )
            cases = @(
                # Route 1: the character is built by a formula.
                @{ formula = '=LEN(UNICHAR(128512))';           tags = @('unicode', 'astral', 'via-unichar') }
                @{ formula = '=LEN(UNICHAR(128512)&"x")';       tags = @('unicode', 'astral', 'via-unichar') }
                @{ formula = '=LEN(UNICHAR(233))';              tags = @('unicode', 'via-unichar') }
                @{ formula = '=LEN(UNICHAR(101)&UNICHAR(769))'; tags = @('unicode', 'via-unichar') }

                # Route 2: the same characters seeded as a cell value.
                @{ formula = '=LEN(A1)';   tags = @('unicode', 'astral', 'via-cell'); note = 'the decisive comparison against LEN(UNICHAR(128512))' }
                @{ formula = '=LEN(A2)';   tags = @('unicode', 'astral', 'via-cell') }
                @{ formula = '=LEN(A3)';   tags = @('unicode', 'via-cell') }
                @{ formula = '=LEN(A4)';   tags = @('unicode', 'via-cell'); note = 'combining mark: two scalars, one grapheme' }
                @{ formula = '=LEN(A5)';   tags = @('unicode', 'via-cell') }
                @{ formula = '=LEN(B1)';   tags = @('unicode', 'via-cell') }
                @{ formula = '=LEN(B3)';   tags = @('unicode', 'astral', 'via-cell'); note = 'written as an explicit surrogate pair rather than as one scalar' }
                @{ formula = '=LEN(B4)';   tags = @('unicode', 'astral', 'via-cell'); note = '3 scalars, 6 UTF-16 code units' }
                @{ formula = '=LEN(B5)';   tags = @('unicode', 'via-cell') }

                # If offsets are scalar-based, these agree with the LEN answers.
                @{ formula = '=LEFT(A2,1)';        tags = @('unicode', 'astral', 'offset'); note = 'does taking one unit split the pair' }
                @{ formula = '=LEN(LEFT(A2,1))';   tags = @('unicode', 'astral', 'offset') }
                @{ formula = '=LEN(RIGHT(A2,1))';  tags = @('unicode', 'astral', 'offset') }
                @{ formula = '=MID(A2,2,1)';       tags = @('unicode', 'astral', 'offset') }
                @{ formula = '=LEN(MID(A2,2,1))';  tags = @('unicode', 'astral', 'offset') }
                @{ formula = '=FIND("x",A2)';      tags = @('unicode', 'astral', 'offset'); note = '2 if scalar-based, 3 if code-unit-based' }
                @{ formula = '=SEARCH("x",A2)';    tags = @('unicode', 'astral', 'offset') }
                @{ formula = '=LEN(MID(B4,2,1))';  tags = @('unicode', 'astral', 'offset') }
                @{ formula = '=UNICODE(A1)';       tags = @('unicode', 'astral'); note = 'the code point Excel reports for the astral character' }
                @{ formula = '=UNICODE(B3)';       tags = @('unicode', 'astral'); note = 'and for the same thing written as a surrogate pair' }
                @{ formula = '=CODE(A1)';          tags = @('unicode', 'astral'); note = 'the legacy single-byte function' }
                @{ formula = '=EXACT(A1,B3)';      tags = @('unicode', 'astral'); note = 'are the two spellings the same string' }
                @{ formula = '=EXACT(A1,UNICHAR(128512))'; tags = @('unicode', 'astral'); note = 'and does UNICHAR produce that same string' }
                @{ formula = '=EXACT(A3,A4)';      tags = @('unicode'); note = 'no NFC normalisation' }

                # TRIM against a non-breaking space, both routes.
                @{ formula = '=LEN(TRIM(B2))';     tags = @('unicode', 'via-cell'); note = 'NBSP is not stripped by TRIM' }
                @{ formula = '=LEN(TRIM(UNICHAR(160)&"abc"&UNICHAR(160)))'; tags = @('unicode', 'via-unichar') }
                @{ formula = '=LEN(CLEAN(B2))';    tags = @('unicode', 'via-cell'); note = 'CLEAN removes control characters, not NBSP' }
                @{ formula = '=LEN(SUBSTITUTE(B2,UNICHAR(160),""))'; tags = @('unicode', 'via-cell'); note = 'the idiom users actually need' }
            )
        }
    )
}
