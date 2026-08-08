# Text functions. The highest-value grid in the corpus, because this is where
# our engine's Rust-native assumptions are most likely to be quietly wrong:
#
#   * LEN of an astral character. Excel counts UTF-16 code units, so an emoji is
#     2. Rust's chars().count() says 1. Every LEFT/RIGHT/MID/FIND offset in the
#     engine inherits whichever answer LEN gives.
#   * TRIM does not remove a non-breaking space, only U+0020.
#   * SEARCH honours * and ? wildcards; FIND does not. Same signature, different
#     language.
#   * UPPER of the German sharp s, where a correct Unicode uppercase is two
#     characters and Excel's may not be.
#
# Non-ASCII inputs are built with UNICHAR() rather than written literally, so
# this grid file is pure ASCII and no encoding guess can corrupt the input half
# of a vector.
@{
    schema = 'ehkatra.oracle.grid/1'
    group  = 'text'

    fixture = @(
        @{ ref = 'A1'; value = 'Hello' }
        @{ ref = 'A2'; value = 'world' }
        @{ ref = 'A3'; blank = $true }
        @{ ref = 'A4'; value = 123 }
        @{ ref = 'A5'; value = $true }
        @{ ref = 'B1'; formula = '=""' }
        @{ ref = 'B2'; value = '  padded  ' }
        @{ ref = 'B3'; text  = '1E2' }
        @{ ref = 'B4'; text  = '$1,234.50' }
        @{ ref = 'B5'; text  = '50%' }
        @{ ref = 'C1'; formula = '=NA()' }
        @{ ref = 'C2'; value = 0.1 }
        @{ ref = 'C3'; formula = '=1/3' }
        @{ ref = 'C4'; value = 1234567890123456 }
        @{ ref = 'C5'; formula = '=DATE(2024,3,15)' }
        @{ ref = 'D1'; text  = '1/2/2024' }
        @{ ref = 'D2'; text  = '  7  ' }
        @{ ref = 'D3'; text  = '(5)' }
        @{ ref = 'D4'; text  = '1 2' }
        @{ ref = 'D5'; text  = '1e2' }
    )

    functions = @(
        @{
            name = 'LEN'
            doc  = 'Counts UTF-16 code units, not scalar values. The astral cases decide whether every string offset in the engine is byte-, char- or code-unit-based.'
            cases = @(
                @{ formula = '=LEN("abc")';            tags = @('basic') }
                @{ formula = '=LEN("")';               tags = @('boundary') }
                @{ formula = '=LEN(A3)';               tags = @('blank') }
                @{ formula = '=LEN(B1)';               tags = @('boundary'); note = 'the ="" cell' }
                @{ formula = '=LEN(A4)';               tags = @('coercion'); note = 'a number coerced to text' }
                @{ formula = '=LEN(A5)';               tags = @('coercion'); note = 'TRUE is four characters' }
                @{ formula = '=LEN(C2)';               tags = @('coercion', 'precision'); note = '0.1 as text -- how many digits' }
                @{ formula = '=LEN(C3)';               tags = @('coercion', 'precision'); note = '1/3 as text -- the 15-digit rule made countable' }
                @{ formula = '=LEN(C4)';               tags = @('coercion', 'precision'); note = 'a 16-digit integer' }
                @{ formula = '=LEN(C5)';               tags = @('coercion'); note = 'a date cell -- serial or formatted text' }
                @{ formula = '=LEN(UNICHAR(233))';     tags = @('unicode'); note = 'e-acute, one code unit' }
                @{ formula = '=LEN(UNICHAR(101)&UNICHAR(769))'; tags = @('unicode'); note = 'e plus combining acute -- two code units, one grapheme' }
                @{ formula = '=LEN(UNICHAR(128512))';  tags = @('unicode', 'astral'); note = 'emoji: 2 in UTF-16, 1 Unicode scalar, 4 UTF-8 bytes' }
                @{ formula = '=LEN(UNICHAR(160))';     tags = @('unicode') }
                @{ formula = '=LEN(UNICHAR(20320)&UNICHAR(22909))'; tags = @('unicode'); note = 'two CJK characters' }
                @{ formula = '=LEN(B2)';               tags = @('basic') }
                @{ formula = '=LEN(C1)';               tags = @('error-input') }
                @{ formula = '=LEN(REPT("a",1000))';   tags = @('boundary') }
            )
        }

        @{
            name = 'LEFT'
            cases = @(
                @{ formula = '=LEFT("abcdef",3)';   tags = @('basic') }
                @{ formula = '=LEFT("abcdef")';     tags = @('argcount'); note = 'omitted count defaults to 1' }
                @{ formula = '=LEFT("abcdef",0)';   tags = @('boundary') }
                @{ formula = '=LEFT("abcdef",-1)';  tags = @('error-input', 'boundary') }
                @{ formula = '=LEFT("abcdef",100)'; tags = @('boundary'); note = 'count past the end' }
                @{ formula = '=LEFT("abcdef",2.9)'; tags = @('coercion'); note = 'fractional count -- truncated or rounded' }
                @{ formula = '=LEFT("abcdef","3")'; tags = @('coercion') }
                @{ formula = '=LEFT(A3,3)';         tags = @('blank') }
                @{ formula = '=LEFT(A4,2)';         tags = @('coercion'); note = 'a number as the source' }
                @{ formula = '=LEFT(UNICHAR(128512)&"x",1)'; tags = @('unicode', 'astral'); note = 'splitting a surrogate pair' }
                @{ formula = '=LEN(LEFT(UNICHAR(128512)&"x",1))'; tags = @('unicode', 'astral'); note = 'what does the half-pair measure' }
                @{ formula = '=LEFT(C1,2)';         tags = @('error-input') }
            )
        }

        @{
            name = 'RIGHT'
            cases = @(
                @{ formula = '=RIGHT("abcdef",3)';   tags = @('basic') }
                @{ formula = '=RIGHT("abcdef")';     tags = @('argcount') }
                @{ formula = '=RIGHT("abcdef",0)';   tags = @('boundary') }
                @{ formula = '=RIGHT("abcdef",-1)';  tags = @('error-input', 'boundary') }
                @{ formula = '=RIGHT("abcdef",100)'; tags = @('boundary') }
                @{ formula = '=RIGHT("abcdef",2.9)'; tags = @('coercion') }
                @{ formula = '=RIGHT(A3,3)';         tags = @('blank') }
                @{ formula = '=RIGHT(A4,2)';         tags = @('coercion') }
                @{ formula = '=RIGHT("x"&UNICHAR(128512),1)'; tags = @('unicode', 'astral') }
                @{ formula = '=RIGHT(C1,2)';         tags = @('error-input') }
            )
        }

        @{
            name = 'MID'
            cases = @(
                @{ formula = '=MID("abcdef",2,3)';   tags = @('basic') }
                @{ formula = '=MID("abcdef",1,0)';   tags = @('boundary') }
                @{ formula = '=MID("abcdef",0,2)';   tags = @('error-input', 'boundary'); note = 'start is 1-based' }
                @{ formula = '=MID("abcdef",-1,2)';  tags = @('error-input') }
                @{ formula = '=MID("abcdef",7,2)';   tags = @('boundary'); note = 'start past the end' }
                @{ formula = '=MID("abcdef",5,100)'; tags = @('boundary') }
                @{ formula = '=MID("abcdef",2,-1)';  tags = @('error-input') }
                @{ formula = '=MID("abcdef",2.9,2)'; tags = @('coercion') }
                @{ formula = '=MID(A3,1,2)';         tags = @('blank') }
                @{ formula = '=MID(A4,2,1)';         tags = @('coercion') }
                @{ formula = '=MID(UNICHAR(128512)&"ab",2,1)'; tags = @('unicode', 'astral') }
                @{ formula = '=MID(C1,1,2)';         tags = @('error-input') }
            )
        }

        @{
            name = 'TRIM'
            doc  = 'Removes leading and trailing U+0020 and collapses internal runs. Does not touch a non-breaking space, which is the single most common cause of a failed lookup on imported data.'
            cases = @(
                @{ formula = '=TRIM("  abc  ")';              tags = @('basic') }
                @{ formula = '=TRIM("a   b")';                tags = @('basic'); note = 'internal run collapses to one' }
                @{ formula = '=TRIM("")';                     tags = @('boundary') }
                @{ formula = '=TRIM("   ")';                  tags = @('boundary') }
                @{ formula = '=TRIM(A3)';                     tags = @('blank') }
                @{ formula = '=TRIM(B2)';                     tags = @('basic') }
                @{ formula = '=LEN(TRIM(UNICHAR(160)&"abc"&UNICHAR(160)))'; tags = @('unicode', 'compat-bug'); note = 'NBSP survives TRIM' }
                @{ formula = '=LEN(TRIM(UNICHAR(9)&"abc"))';  tags = @('unicode'); note = 'tab' }
                @{ formula = '=LEN(TRIM(UNICHAR(10)&"abc"))'; tags = @('unicode'); note = 'line feed' }
                @{ formula = '=TRIM(A4)';                     tags = @('coercion') }
                @{ formula = '=TRIM(C1)';                     tags = @('error-input') }
            )
        }

        @{
            name = 'UPPER'
            cases = @(
                @{ formula = '=UPPER("abc")';                 tags = @('basic') }
                @{ formula = '=UPPER("aBc123")';              tags = @('basic') }
                @{ formula = '=UPPER("")';                    tags = @('boundary') }
                @{ formula = '=UPPER(A3)';                    tags = @('blank') }
                @{ formula = '=UPPER(A4)';                    tags = @('coercion') }
                @{ formula = '=UPPER(UNICHAR(223))';          tags = @('unicode'); note = 'sharp s: a correct uppercase is two characters' }
                @{ formula = '=LEN(UPPER(UNICHAR(223)))';     tags = @('unicode'); note = 'does the length change' }
                @{ formula = '=UPPER(UNICHAR(233))';          tags = @('unicode') }
                @{ formula = '=UPPER(UNICHAR(305))';          tags = @('unicode'); note = 'dotless i -- the Turkish case' }
                @{ formula = '=UPPER(UNICHAR(20320))';        tags = @('unicode'); note = 'CJK has no case' }
                @{ formula = '=UPPER(C1)';                    tags = @('error-input') }
            )
        }

        @{
            name = 'LOWER'
            cases = @(
                @{ formula = '=LOWER("ABC")';              tags = @('basic') }
                @{ formula = '=LOWER("AbC123")';           tags = @('basic') }
                @{ formula = '=LOWER("")';                 tags = @('boundary') }
                @{ formula = '=LOWER(A3)';                 tags = @('blank') }
                @{ formula = '=LOWER(A4)';                 tags = @('coercion') }
                @{ formula = '=LOWER(UNICHAR(304))';       tags = @('unicode'); note = 'I with dot above' }
                @{ formula = '=LEN(LOWER(UNICHAR(304)))';  tags = @('unicode') }
                @{ formula = '=LOWER(UNICHAR(931))';       tags = @('unicode'); note = 'Greek capital sigma -- final versus medial lowercase' }
                @{ formula = '=LOWER(C1)';                 tags = @('error-input') }
            )
        }

        @{
            name = 'PROPER'
            doc  = 'Word boundaries are defined by "not a letter", so digits and apostrophes start a new word. Both are surprising and both matter for name data.'
            cases = @(
                @{ formula = '=PROPER("hello world")';   tags = @('basic') }
                @{ formula = '=PROPER("HELLO WORLD")';   tags = @('basic') }
                @{ formula = '=PROPER("o''brien")';      tags = @('compat-bug'); note = 'apostrophe as a word boundary' }
                @{ formula = '=PROPER("2nd place")';     tags = @('compat-bug'); note = 'a digit starts a word, so the n capitalises' }
                @{ formula = '=PROPER("a1b2c3")';        tags = @('compat-bug') }
                @{ formula = '=PROPER("mcdonald-smith")'; tags = @('basic') }
                @{ formula = '=PROPER("")';              tags = @('boundary') }
                @{ formula = '=PROPER(A3)';              tags = @('blank') }
                @{ formula = '=PROPER(A4)';              tags = @('coercion') }
                @{ formula = '=PROPER("e"&UNICHAR(769)&"cole")'; tags = @('unicode'); note = 'combining mark after the first letter' }
                @{ formula = '=PROPER(C1)';              tags = @('error-input') }
            )
        }

        @{
            name = 'SUBSTITUTE'
            doc  = 'Case-sensitive, no wildcards, and the fourth argument counts occurrences rather than limiting them.'
            cases = @(
                @{ formula = '=SUBSTITUTE("banana","a","X")';       tags = @('basic') }
                @{ formula = '=SUBSTITUTE("banana","a","X",2)';     tags = @('basic'); note = 'the second occurrence only' }
                @{ formula = '=SUBSTITUTE("banana","a","X",0)';     tags = @('error-input', 'boundary') }
                @{ formula = '=SUBSTITUTE("banana","a","X",9)';     tags = @('boundary'); note = 'occurrence beyond the count' }
                @{ formula = '=SUBSTITUTE("banana","A","X")';       tags = @('basic'); note = 'case-sensitive: no match' }
                @{ formula = '=SUBSTITUTE("banana","","X")';        tags = @('boundary'); note = 'empty needle' }
                @{ formula = '=SUBSTITUTE("banana","a","")';        tags = @('boundary'); note = 'deletion' }
                @{ formula = '=SUBSTITUTE("banana","*","X")';       tags = @('boundary'); note = 'no wildcards here' }
                @{ formula = '=SUBSTITUTE("aaa","aa","X")';         tags = @('boundary'); note = 'overlapping matches' }
                @{ formula = '=SUBSTITUTE("","","")';               tags = @('boundary') }
                @{ formula = '=SUBSTITUTE(A3,"a","X")';             tags = @('blank') }
                @{ formula = '=SUBSTITUTE(A4,"2","X")';             tags = @('coercion') }
                @{ formula = '=SUBSTITUTE("banana","a","X",2.9)';   tags = @('coercion') }
                @{ formula = '=SUBSTITUTE(C1,"a","X")';             tags = @('error-input') }
            )
        }

        @{
            name = 'REPLACE'
            cases = @(
                @{ formula = '=REPLACE("abcdef",2,3,"XY")';  tags = @('basic') }
                @{ formula = '=REPLACE("abcdef",1,0,"XY")';  tags = @('boundary'); note = 'zero length is an insertion' }
                @{ formula = '=REPLACE("abcdef",0,2,"XY")';  tags = @('error-input') }
                @{ formula = '=REPLACE("abcdef",-1,2,"XY")'; tags = @('error-input') }
                @{ formula = '=REPLACE("abcdef",2,-1,"XY")'; tags = @('error-input') }
                @{ formula = '=REPLACE("abcdef",7,2,"XY")';  tags = @('boundary'); note = 'start past the end appends' }
                @{ formula = '=REPLACE("abcdef",3,100,"XY")'; tags = @('boundary') }
                @{ formula = '=REPLACE("abcdef",2,3,"")';    tags = @('boundary'); note = 'deletion' }
                @{ formula = '=REPLACE(A3,1,1,"X")';         tags = @('blank') }
                @{ formula = '=REPLACE(A4,1,1,"X")';         tags = @('coercion') }
                @{ formula = '=REPLACE(UNICHAR(128512)&"ab",1,1,"X")'; tags = @('unicode', 'astral') }
                @{ formula = '=REPLACE(C1,1,1,"X")';         tags = @('error-input') }
            )
        }

        @{
            name = 'FIND'
            doc  = 'Case-sensitive and literal. No wildcards, which is the only thing separating it from SEARCH.'
            cases = @(
                @{ formula = '=FIND("b","abcabc")';      tags = @('basic') }
                @{ formula = '=FIND("b","abcabc",3)';    tags = @('basic'); note = 'start offset' }
                @{ formula = '=FIND("B","abcabc")';      tags = @('basic'); note = 'case-sensitive: no match' }
                @{ formula = '=FIND("","abc")';          tags = @('boundary'); note = 'empty needle' }
                @{ formula = '=FIND("","abc",2)';        tags = @('boundary') }
                @{ formula = '=FIND("a","")';            tags = @('boundary') }
                @{ formula = '=FIND("z","abc")';         tags = @('error-input') }
                @{ formula = '=FIND("a","abc",0)';       tags = @('error-input', 'boundary') }
                @{ formula = '=FIND("a","abc",10)';      tags = @('error-input', 'boundary') }
                @{ formula = '=FIND("a*c","abc")';       tags = @('boundary'); note = 'no wildcards -- compare with SEARCH' }
                @{ formula = '=FIND("b",UNICHAR(128512)&"ab")'; tags = @('unicode', 'astral'); note = 'is the returned offset in code units' }
                @{ formula = '=FIND("a",A4)';            tags = @('coercion') }
                @{ formula = '=FIND("a",A3)';            tags = @('blank') }
                @{ formula = '=FIND("a",C1)';            tags = @('error-input') }
            )
        }

        @{
            name = 'SEARCH'
            doc  = 'Case-insensitive AND wildcard-aware: * ? and the ~ escape. A conformance gap here silently changes lookup semantics.'
            cases = @(
                @{ formula = '=SEARCH("b","abcabc")';     tags = @('basic') }
                @{ formula = '=SEARCH("B","abcabc")';     tags = @('basic'); note = 'case-insensitive' }
                @{ formula = '=SEARCH("b","abcabc",3)';   tags = @('basic') }
                @{ formula = '=SEARCH("a*c","abxc")';     tags = @('wildcard'); note = '* matches any run' }
                @{ formula = '=SEARCH("a?c","abc")';      tags = @('wildcard'); note = '? matches exactly one' }
                @{ formula = '=SEARCH("a?c","ac")';       tags = @('wildcard') }
                @{ formula = '=SEARCH("~*","a*b")';       tags = @('wildcard'); note = 'the tilde escape' }
                @{ formula = '=SEARCH("*","abc")';        tags = @('wildcard', 'boundary') }
                @{ formula = '=SEARCH("","abc")';         tags = @('boundary') }
                @{ formula = '=SEARCH("z","abc")';        tags = @('error-input') }
                @{ formula = '=SEARCH("a","abc",0)';      tags = @('error-input', 'boundary') }
                @{ formula = '=SEARCH("a",A3)';           tags = @('blank') }
                @{ formula = '=SEARCH("a",C1)';           tags = @('error-input') }
            )
        }

        @{
            name = 'REPT'
            cases = @(
                @{ formula = '=REPT("ab",3)';        tags = @('basic') }
                @{ formula = '=REPT("ab",0)';        tags = @('boundary') }
                @{ formula = '=REPT("ab",-1)';       tags = @('error-input') }
                @{ formula = '=REPT("",5)';          tags = @('boundary') }
                @{ formula = '=REPT("ab",2.9)';      tags = @('coercion'); note = 'fractional count' }
                @{ formula = '=LEN(REPT("a",32767))'; tags = @('boundary'); note = 'the cell character limit' }
                @{ formula = '=LEN(REPT("a",32768))'; tags = @('boundary', 'overflow'); note = 'one past it' }
                @{ formula = '=REPT(A4,2)';          tags = @('coercion') }
                @{ formula = '=REPT(A3,2)';          tags = @('blank') }
                @{ formula = '=REPT(C1,2)';          tags = @('error-input') }
            )
        }

        @{
            name = 'EXACT'
            doc  = 'The only text comparison in Excel that is case-sensitive; = is not.'
            cases = @(
                @{ formula = '=EXACT("a","a")';       tags = @('basic') }
                @{ formula = '=EXACT("a","A")';       tags = @('basic'); note = 'contrast with ="a"="A"' }
                @{ formula = '=EXACT("","")';         tags = @('boundary') }
                @{ formula = '=EXACT(A3,"")';         tags = @('blank'); note = 'blank against an empty string' }
                @{ formula = '=EXACT(A3,A3)';         tags = @('blank') }
                @{ formula = '=EXACT(1,"1")';         tags = @('coercion') }
                @{ formula = '=EXACT(1,1)';           tags = @('basic') }
                @{ formula = '=EXACT(TRUE,"TRUE")';   tags = @('coercion') }
                @{ formula = '=EXACT("a "&"","a")';   tags = @('boundary'); note = 'trailing space is significant' }
                @{ formula = '=EXACT(UNICHAR(233),UNICHAR(101)&UNICHAR(769))'; tags = @('unicode'); note = 'no NFC normalisation' }
                @{ formula = '=EXACT(C1,C1)';         tags = @('error-input') }
            )
        }

        @{
            name = 'CONCAT'
            doc  = 'Accepts ranges, unlike CONCATENATE. Blanks vanish, errors propagate.'
            cases = @(
                @{ formula = '=CONCAT("a","b","c")';   tags = @('basic') }
                @{ formula = '=CONCAT(A1:A2)';         tags = @('range') }
                @{ formula = '=CONCAT(A1:A5)';         tags = @('range', 'coercion'); note = 'blank, number and logical in one range' }
                @{ formula = '=CONCAT("")';            tags = @('boundary') }
                @{ formula = '=CONCAT(A3)';            tags = @('blank') }
                @{ formula = '=CONCAT(C2)';            tags = @('precision'); note = '0.1 spliced into text' }
                @{ formula = '=CONCAT(C3)';            tags = @('precision', 'compat-bug'); note = '1/3 -- the 15-digit display rule in text form' }
                @{ formula = '=CONCAT(C4)';            tags = @('precision', 'compat-bug'); note = 'a 16-digit integer' }
                @{ formula = '=CONCAT(C5)';            tags = @('boundary'); note = 'a date becomes its serial, not its display' }
                @{ formula = '=CONCAT(A5)';            tags = @('coercion') }
                @{ formula = '=CONCAT(C1)';            tags = @('error-input') }
            )
        }

        @{
            name = 'CONCATENATE'
            doc  = 'The legacy spelling. Whether it accepts a range at all is version-dependent, and Excel may rewrite the stored formula with an implicit-intersection @.'
            cases = @(
                @{ formula = '=CONCATENATE("a","b","c")'; tags = @('basic') }
                @{ formula = '=CONCATENATE("a")';         tags = @('argcount') }
                @{ formula = '=CONCATENATE(A1,A2)';       tags = @('basic') }
                @{ formula = '=CONCATENATE(A1,A3,A2)';    tags = @('blank') }
                @{ formula = '=CONCATENATE(A4,A5)';       tags = @('coercion') }
                @{ formula = '=CONCATENATE(A1:A2)';       tags = @('range', 'boundary'); note = 'a range argument -- expect an implicit-intersection rewrite' }
                @{ formula = '=CONCATENATE(C2,"")';       tags = @('precision') }
                @{ formula = '=CONCATENATE(C1,"a")';      tags = @('error-input') }
            )
        }

        @{
            name = 'TEXTJOIN'
            cases = @(
                @{ formula = '=TEXTJOIN(",",TRUE,"a","b","c")';  tags = @('basic') }
                @{ formula = '=TEXTJOIN(",",TRUE,A1:A5)';        tags = @('range'); note = 'blank skipped' }
                @{ formula = '=TEXTJOIN(",",FALSE,A1:A5)';       tags = @('range'); note = 'blank kept as an empty field' }
                @{ formula = '=TEXTJOIN("",TRUE,A1:A2)';         tags = @('boundary'); note = 'empty delimiter' }
                @{ formula = '=TEXTJOIN(",",TRUE,"")';           tags = @('boundary') }
                @{ formula = '=TEXTJOIN(",",FALSE,"","")';       tags = @('boundary') }
                @{ formula = '=TEXTJOIN(",",1,"a","b")';         tags = @('coercion'); note = 'numeric ignore_empty' }
                @{ formula = '=TEXTJOIN(",","x","a","b")';       tags = @('error-input') }
                @{ formula = '=TEXTJOIN(",",TRUE,A3)';           tags = @('blank') }
                @{ formula = '=TEXTJOIN(",",TRUE,C1)';           tags = @('error-input') }
                @{ formula = '=TEXTJOIN(",",TRUE,C4)';           tags = @('precision') }
            )
        }

        @{
            name = 'VALUE'
            doc  = 'The explicit text-to-number conversion, and therefore the definitive statement of what Excel considers numeric text -- currency symbols, thousands separators, percent signs, parenthesised negatives and dates all included.'
            cases = @(
                @{ formula = '=VALUE("123")';        tags = @('basic') }
                @{ formula = '=VALUE("-123")';       tags = @('basic') }
                @{ formula = '=VALUE("1E2")';        tags = @('coercion', 'compat-bug'); note = 'the gene-symbol mangling' }
                @{ formula = '=VALUE(D5)';           tags = @('coercion'); note = 'lowercase e exponent' }
                @{ formula = '=VALUE(B4)';           tags = @('coercion'); note = 'currency and thousands separators' }
                @{ formula = '=VALUE(B5)';           tags = @('coercion'); note = 'percent -- is it divided by 100' }
                @{ formula = '=VALUE(D2)';           tags = @('coercion'); note = 'surrounding spaces' }
                @{ formula = '=VALUE(D3)';           tags = @('coercion'); note = 'parenthesised negative' }
                @{ formula = '=VALUE(D4)';           tags = @('error-input'); note = 'a space inside the digits' }
                @{ formula = '=VALUE(D1)';           tags = @('coercion'); note = 'a date string becomes its serial' }
                @{ formula = '=VALUE("")';           tags = @('error-input', 'boundary') }
                @{ formula = '=VALUE(A3)';           tags = @('blank') }
                @{ formula = '=VALUE("abc")';        tags = @('error-input') }
                @{ formula = '=VALUE(TRUE)';         tags = @('error-input'); note = 'a logical is not numeric text' }
                @{ formula = '=VALUE(123)';          tags = @('basic') }
                @{ formula = '=VALUE("0x10")';       tags = @('error-input') }
                @{ formula = '=VALUE("1.7976931348623157E+308")'; tags = @('boundary') }
                @{ formula = '=VALUE("1E+309")';     tags = @('boundary', 'overflow') }
                @{ formula = '=VALUE("Infinity")';   tags = @('error-input') }
                @{ formula = '=VALUE(C1)';           tags = @('error-input') }
            )
        }
    )
}
