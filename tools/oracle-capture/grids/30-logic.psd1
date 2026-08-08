# Logical functions and type predicates. Two things are being measured here.
#
# First, truthiness: Excel coerces a literal text "TRUE" but skips text inside a
# range, and AND over a range with no logicals at all is #VALUE! rather than the
# vacuous truth a fold would give.
#
# Second, the blank/empty-string boundary. A cell holding ="" is simultaneously
# not blank (ISBLANK is FALSE), non-empty (COUNTA counts it) and blank
# (COUNTBLANK counts it). Three functions, three answers, one cell.
@{
    schema = 'ehkatra.oracle.grid/1'
    group  = 'logic'

    fixture = @(
        @{ ref = 'A1'; blank = $true }
        @{ ref = 'A2'; value = 0 }
        @{ ref = 'A3'; value = 1 }
        @{ ref = 'A4'; value = $true }
        @{ ref = 'A5'; value = $false }
        @{ ref = 'B1'; formula = '=""' }
        @{ ref = 'B2'; value = 'abc' }
        @{ ref = 'B3'; text  = 'TRUE' }
        @{ ref = 'B4'; text  = '0' }
        @{ ref = 'B5'; value = ' ' }
        @{ ref = 'C1'; formula = '=NA()' }
        @{ ref = 'C2'; formula = '=1/0' }
        @{ ref = 'C3'; formula = '=SQRT(-1)' }
        # A range that mixes logicals with text and blanks
        @{ ref = 'D1'; value = $true }
        @{ ref = 'D2'; value = 'x' }
        @{ ref = 'D3'; blank = $true }
        @{ ref = 'D4'; value = $false }
        # A range with no logicals or numbers at all
        @{ ref = 'E1'; value = 'p' }
        @{ ref = 'E2'; value = 'q' }
    )

    functions = @(
        @{
            name = 'IF'
            doc  = 'Condition truthiness, and the omitted-branch defaults: a missing value_if_false is FALSE, not blank.'
            cases = @(
                @{ formula = '=IF(TRUE,1,2)';        tags = @('basic') }
                @{ formula = '=IF(FALSE,1,2)';       tags = @('basic') }
                @{ formula = '=IF(1,1,2)';           tags = @('coercion') }
                @{ formula = '=IF(0,1,2)';           tags = @('coercion') }
                @{ formula = '=IF(-1,1,2)';          tags = @('coercion'); note = 'any non-zero is true' }
                @{ formula = '=IF("TRUE",1,2)';      tags = @('coercion'); note = 'literal text that spells a logical' }
                @{ formula = '=IF("true",1,2)';      tags = @('coercion'); note = 'case sensitivity of that coercion' }
                @{ formula = '=IF("1",1,2)';         tags = @('coercion'); note = 'numeric text as a condition' }
                @{ formula = '=IF("abc",1,2)';       tags = @('error-input') }
                @{ formula = '=IF(A1,1,2)';          tags = @('blank'); note = 'blank condition' }
                @{ formula = '=IF(B1,1,2)';          tags = @('boundary'); note = 'a formula-produced empty string as a condition' }
                @{ formula = '=IF(C1,1,2)';          tags = @('error-input') }
                @{ formula = '=IF(TRUE,"a")';        tags = @('argcount') }
                @{ formula = '=IF(FALSE,"a")';       tags = @('argcount'); note = 'omitted false branch' }
                @{ formula = '=IF(TRUE,,2)';         tags = @('argcount', 'boundary'); note = 'empty argument, not omitted' }
                @{ formula = '=IF(FALSE,1,)';        tags = @('argcount', 'boundary') }
                @{ formula = '=IF(TRUE,1/0,2)';      tags = @('lazy'); note = 'the taken branch errors' }
                @{ formula = '=IF(FALSE,1/0,2)';     tags = @('lazy'); note = 'the untaken branch must not be evaluated' }
                @{ formula = '=IF(TRUE,A1,2)';       tags = @('blank'); note = 'returning a blank cell' }
                @{ formula = '=IF(B3,1,2)';          tags = @('coercion'); note = 'cell containing the text TRUE' }
            )
        }

        @{
            name = 'IFS'
            doc  = 'No else branch: falling off the end is #N/A.'
            cases = @(
                @{ formula = '=IFS(TRUE,1)';                tags = @('basic') }
                @{ formula = '=IFS(FALSE,1,TRUE,2)';        tags = @('basic') }
                @{ formula = '=IFS(FALSE,1)';               tags = @('boundary'); note = 'no condition matched' }
                @{ formula = '=IFS(FALSE,1,FALSE,2)';       tags = @('boundary') }
                @{ formula = '=IFS(TRUE,1,TRUE,2)';         tags = @('basic'); note = 'first match wins' }
                @{ formula = '=IFS(1,"a")';                 tags = @('coercion') }
                @{ formula = '=IFS(A1,"a",TRUE,"b")';       tags = @('blank') }
                @{ formula = '=IFS(C1,1,TRUE,2)';           tags = @('error-input') }
                @{ formula = '=IFS(FALSE,1/0,TRUE,2)';      tags = @('lazy') }
                @{ formula = '=IFS("abc",1,TRUE,2)';        tags = @('error-input') }
            )
        }

        @{
            name = 'AND'
            doc  = 'Skips text and blanks inside a range; rejects text as a literal argument. All-skipped is #VALUE!, not TRUE.'
            cases = @(
                @{ formula = '=AND(TRUE,TRUE)';    tags = @('basic') }
                @{ formula = '=AND(TRUE,FALSE)';   tags = @('basic') }
                @{ formula = '=AND(1,1)';          tags = @('coercion') }
                @{ formula = '=AND(1,0)';          tags = @('coercion') }
                @{ formula = '=AND(TRUE,"x")';     tags = @('error-input'); note = 'text as a literal argument' }
                @{ formula = '=AND(D1:D4)';        tags = @('range'); note = 'text and blank in the range are skipped' }
                @{ formula = '=AND(E1:E2)';        tags = @('range', 'boundary'); note = 'nothing in the range is truthy-able' }
                @{ formula = '=AND(A1)';           tags = @('blank', 'boundary') }
                @{ formula = '=AND(B1)';           tags = @('boundary'); note = 'the ="" cell' }
                @{ formula = '=AND(C1,TRUE)';      tags = @('error-input') }
                @{ formula = '=AND(TRUE)';         tags = @('argcount') }
                @{ formula = '=AND(A2:A5)';        tags = @('range'); note = '0, 1, TRUE, FALSE' }
            )
        }

        @{
            name = 'OR'
            cases = @(
                @{ formula = '=OR(FALSE,TRUE)';   tags = @('basic') }
                @{ formula = '=OR(FALSE,FALSE)';  tags = @('basic') }
                @{ formula = '=OR(0,0)';          tags = @('coercion') }
                @{ formula = '=OR(0,-1)';         tags = @('coercion') }
                @{ formula = '=OR(FALSE,"x")';    tags = @('error-input') }
                @{ formula = '=OR(D1:D4)';        tags = @('range') }
                @{ formula = '=OR(E1:E2)';        tags = @('range', 'boundary') }
                @{ formula = '=OR(A1)';           tags = @('blank', 'boundary') }
                @{ formula = '=OR(C1,TRUE)';      tags = @('error-input') }
                @{ formula = '=OR(A2:A5)';        tags = @('range') }
            )
        }

        @{
            name = 'NOT'
            cases = @(
                @{ formula = '=NOT(TRUE)';    tags = @('basic') }
                @{ formula = '=NOT(FALSE)';   tags = @('basic') }
                @{ formula = '=NOT(0)';       tags = @('coercion') }
                @{ formula = '=NOT(1)';       tags = @('coercion') }
                @{ formula = '=NOT(2)';       tags = @('coercion') }
                @{ formula = '=NOT("abc")';   tags = @('error-input') }
                @{ formula = '=NOT("TRUE")';  tags = @('coercion') }
                @{ formula = '=NOT(A1)';      tags = @('blank') }
                @{ formula = '=NOT(B1)';      tags = @('boundary') }
                @{ formula = '=NOT(C1)';      tags = @('error-input') }
            )
        }

        @{
            name = 'XOR'
            doc  = 'Parity of the true values, so it is not a two-argument operator.'
            cases = @(
                @{ formula = '=XOR(TRUE,TRUE)';        tags = @('basic') }
                @{ formula = '=XOR(TRUE,FALSE)';       tags = @('basic') }
                @{ formula = '=XOR(TRUE,TRUE,TRUE)';   tags = @('basic'); note = 'parity, not pairwise' }
                @{ formula = '=XOR(1,1,1,1)';          tags = @('coercion') }
                @{ formula = '=XOR(FALSE,FALSE)';      tags = @('basic') }
                @{ formula = '=XOR(D1:D4)';            tags = @('range') }
                @{ formula = '=XOR(E1:E2)';            tags = @('range', 'boundary') }
                @{ formula = '=XOR(A1)';               tags = @('blank', 'boundary') }
                @{ formula = '=XOR(C1,TRUE)';          tags = @('error-input') }
                @{ formula = '=XOR(TRUE)';             tags = @('argcount') }
            )
        }

        @{
            name = 'IFERROR'
            doc  = 'Catches every error class. Must not let the first argument propagate, which means it evaluates lazily.'
            cases = @(
                @{ formula = '=IFERROR(1/0,"caught")';    tags = @('basic') }
                @{ formula = '=IFERROR(NA(),"caught")';   tags = @('basic') }
                @{ formula = '=IFERROR(SQRT(-1),"caught")'; tags = @('basic') }
                @{ formula = '=IFERROR(1,"caught")';      tags = @('basic') }
                @{ formula = '=IFERROR("",1)';            tags = @('boundary'); note = 'an empty string is not an error' }
                @{ formula = '=IFERROR(A1,"caught")';     tags = @('blank'); note = 'a blank first argument' }
                @{ formula = '=IFERROR(1/0,)';            tags = @('argcount', 'boundary'); note = 'empty fallback' }
                @{ formula = '=IFERROR(1/0,1/0)';         tags = @('boundary'); note = 'the fallback errors too' }
                @{ formula = '=IFERROR(C1,C2)';           tags = @('error-input') }
                @{ formula = '=IFERROR(UNKNOWNFUNC(),"caught")'; tags = @('error-input'); note = 'is #NAME? catchable' }
            )
        }

        @{
            name = 'IFNA'
            doc  = 'Catches #N/A only, so every other error class must pass straight through.'
            cases = @(
                @{ formula = '=IFNA(NA(),"caught")';       tags = @('basic') }
                @{ formula = '=IFNA(1/0,"caught")';        tags = @('basic'); note = '#DIV/0! must not be caught' }
                @{ formula = '=IFNA(SQRT(-1),"caught")';   tags = @('basic') }
                @{ formula = '=IFNA(1,"caught")';          tags = @('basic') }
                @{ formula = '=IFNA(A1,"caught")';         tags = @('blank') }
                @{ formula = '=IFNA(VLOOKUP("zz",A1:A5,1,FALSE),"caught")'; tags = @('basic'); note = 'the case it exists for' }
                @{ formula = '=IFNA(NA(),)';               tags = @('argcount', 'boundary') }
                @{ formula = '=IFNA(C1,"caught")';         tags = @('error-input') }
            )
        }

        @{
            name = 'ISERROR'
            cases = @(
                @{ formula = '=ISERROR(1/0)';      tags = @('basic') }
                @{ formula = '=ISERROR(NA())';     tags = @('basic'); note = '#N/A counts as an error here, unlike ISERR' }
                @{ formula = '=ISERROR(1)';        tags = @('basic') }
                @{ formula = '=ISERROR("abc")';    tags = @('basic') }
                @{ formula = '=ISERROR(A1)';       tags = @('blank') }
                @{ formula = '=ISERROR(B1)';       tags = @('boundary') }
                @{ formula = '=ISERROR(C1:C3)';    tags = @('range'); note = 'a range argument to a scalar predicate' }
                @{ formula = '=ISERROR(UNKNOWNFUNC())'; tags = @('basic') }
            )
        }

        @{
            name = 'ISNA'
            cases = @(
                @{ formula = '=ISNA(NA())';     tags = @('basic') }
                @{ formula = '=ISNA(1/0)';      tags = @('basic') }
                @{ formula = '=ISNA(1)';        tags = @('basic') }
                @{ formula = '=ISNA(A1)';       tags = @('blank') }
                @{ formula = '=ISNA(C1)';       tags = @('basic') }
                @{ formula = '=ISNA(C2)';       tags = @('basic') }
                @{ formula = '=ISNA("#N/A")';   tags = @('boundary'); note = 'the text spelling of the error is not the error' }
            )
        }

        @{
            name = 'ISBLANK'
            doc  = 'True only for a genuinely empty cell. The ="" cell is where ISBLANK and COUNTBLANK part company.'
            cases = @(
                @{ formula = '=ISBLANK(A1)';    tags = @('basic') }
                @{ formula = '=ISBLANK(A2)';    tags = @('basic') }
                @{ formula = '=ISBLANK(B1)';    tags = @('compat-bug', 'boundary'); note = 'the ="" cell: FALSE here, counted by COUNTBLANK' }
                @{ formula = '=ISBLANK("")';    tags = @('boundary'); note = 'a literal empty string' }
                @{ formula = '=ISBLANK(0)';     tags = @('basic') }
                @{ formula = '=ISBLANK(C1)';    tags = @('error-input') }
                @{ formula = '=ISBLANK(B5)';    tags = @('boundary'); note = 'a cell containing one space' }
            )
        }

        @{
            name = 'ISNUMBER'
            cases = @(
                @{ formula = '=ISNUMBER(1)';       tags = @('basic') }
                @{ formula = '=ISNUMBER(-1.5)';    tags = @('basic') }
                @{ formula = '=ISNUMBER("1")';     tags = @('coercion'); note = 'no coercion in a predicate' }
                @{ formula = '=ISNUMBER(B4)';      tags = @('coercion'); note = 'a cell holding the text "0"' }
                @{ formula = '=ISNUMBER(TRUE)';    tags = @('basic') }
                @{ formula = '=ISNUMBER(A1)';      tags = @('blank') }
                @{ formula = '=ISNUMBER(C1)';      tags = @('error-input') }
                @{ formula = '=ISNUMBER(DATE(2024,1,1))'; tags = @('basic'); note = 'a date is a number' }
                @{ formula = '=ISNUMBER(1/0)';     tags = @('error-input') }
            )
        }

        @{
            name = 'ISTEXT'
            cases = @(
                @{ formula = '=ISTEXT("abc")';  tags = @('basic') }
                @{ formula = '=ISTEXT("")';     tags = @('boundary') }
                @{ formula = '=ISTEXT(B1)';     tags = @('boundary'); note = 'the ="" cell is text' }
                @{ formula = '=ISTEXT(1)';      tags = @('basic') }
                @{ formula = '=ISTEXT(B4)';     tags = @('basic'); note = 'text that looks numeric' }
                @{ formula = '=ISTEXT(TRUE)';   tags = @('basic') }
                @{ formula = '=ISTEXT(A1)';     tags = @('blank') }
                @{ formula = '=ISTEXT(C1)';     tags = @('error-input') }
            )
        }

        @{
            name = 'ISLOGICAL'
            cases = @(
                @{ formula = '=ISLOGICAL(TRUE)';    tags = @('basic') }
                @{ formula = '=ISLOGICAL(FALSE)';   tags = @('basic') }
                @{ formula = '=ISLOGICAL(1)';       tags = @('basic') }
                @{ formula = '=ISLOGICAL("TRUE")';  tags = @('coercion') }
                @{ formula = '=ISLOGICAL(B3)';      tags = @('coercion'); note = 'a cell holding the text TRUE' }
                @{ formula = '=ISLOGICAL(A4)';      tags = @('basic') }
                @{ formula = '=ISLOGICAL(A1)';      tags = @('blank') }
                @{ formula = '=ISLOGICAL(C1)';      tags = @('error-input') }
            )
        }

        @{
            name = 'NA'
            doc  = 'The only function whose job is to produce an error, so error identity is what gets recorded.'
            cases = @(
                @{ formula = '=NA()';              tags = @('basic') }
                @{ formula = '=ISNA(NA())';        tags = @('basic') }
                @{ formula = '=NA()&""';           tags = @('error-input'); note = 'errors win over concatenation' }
                @{ formula = '=NA()+1';            tags = @('error-input') }
                @{ formula = '=COUNT(NA())';       tags = @('error-input') }
                @{ formula = '=ERROR.TYPE(NA())';  tags = @('basic'); note = 'pins the error class number itself' }
            )
        }
    )
}
