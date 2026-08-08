# Lookup and conditional aggregation.
#
# The engine currently implements exact match only and rejects an explicit
# range_lookup TRUE rather than faking a binary search (functions.rs f_lookup_
# table). That is a defensible v0.1 choice, but it is a *divergence*, and a
# divergence you have not measured is a bug you have not found. So the
# approximate-match cases are captured deliberately, including the ones over
# unsorted data where Excel's answer is defined by its algorithm rather than by
# its documentation.
#
# The criteria language (>5, <>x, wildcards, a ~ escape, a cell reference) is a
# second sub-language inside Excel with its own coercion rules. It gets the same
# treatment.
@{
    schema = 'ehkatra.oracle.grid/1'
    group  = 'lookup'

    fixture = @(
        # A:B a sorted numeric key table
        @{ ref = 'A1'; value = 10 }
        @{ ref = 'A2'; value = 20 }
        @{ ref = 'A3'; value = 30 }
        @{ ref = 'A4'; value = 40 }
        @{ ref = 'A5'; value = 50 }
        @{ ref = 'B1'; value = 'ten' }
        @{ ref = 'B2'; value = 'twenty' }
        @{ ref = 'B3'; value = 'thirty' }
        @{ ref = 'B4'; value = 'forty' }
        @{ ref = 'B5'; value = 'fifty' }
        # C:D an unsorted key table, with a duplicate and a blank
        @{ ref = 'C1'; value = 30 }
        @{ ref = 'C2'; value = 10 }
        @{ ref = 'C3'; value = 50 }
        @{ ref = 'C4'; value = 10 }
        @{ ref = 'C5'; blank = $true }
        @{ ref = 'D1'; value = 'c-thirty' }
        @{ ref = 'D2'; value = 'c-ten-first' }
        @{ ref = 'D3'; value = 'c-fifty' }
        @{ ref = 'D4'; value = 'c-ten-second' }
        @{ ref = 'D5'; value = 'c-blank' }
        # E:F a text key table, mixed case, with a numeric-looking text key
        @{ ref = 'E1'; value = 'apple' }
        @{ ref = 'E2'; value = 'Banana' }
        @{ ref = 'E3'; value = 'cherry' }
        @{ ref = 'E4'; text  = '7' }
        @{ ref = 'E5'; value = 'a*c' }
        @{ ref = 'F1'; value = 1 }
        @{ ref = 'F2'; value = 2 }
        @{ ref = 'F3'; value = 3 }
        @{ ref = 'F4'; value = 4 }
        @{ ref = 'F5'; value = 5 }
        # G a horizontal table for HLOOKUP: G1:G3 keys, G4:G6 unused; see block below
        @{ ref = 'G1'; value = 10 }
        @{ ref = 'G2'; value = 'g-ten' }
        @{ ref = 'G3'; formula = '=NA()' }
        # H the amounts column that SUMIF sums, deliberately misaligned in type
        @{ ref = 'H1'; value = 100 }
        @{ ref = 'H2'; value = 200 }
        @{ ref = 'H3'; text  = '300' }
        @{ ref = 'H4'; value = 400 }
        @{ ref = 'H5'; value = -50 }
    )

    functions = @(
        @{
            name = 'VLOOKUP'
            doc  = 'Exact and approximate match, over sorted and unsorted keys. Text keys compare case-insensitively and honour wildcards; the column index is 1-based and out of range is #REF!, not #VALUE!.'
            cases = @(
                @{ formula = '=VLOOKUP(30,A1:B5,2,FALSE)';    tags = @('basic', 'exact') }
                @{ formula = '=VLOOKUP(30,A1:B5,1,FALSE)';    tags = @('exact'); note = 'returning the key column' }
                @{ formula = '=VLOOKUP(35,A1:B5,2,FALSE)';    tags = @('exact'); note = 'no exact match' }
                @{ formula = '=VLOOKUP(35,A1:B5,2,TRUE)';     tags = @('approximate'); note = 'largest key not greater than 35' }
                @{ formula = '=VLOOKUP(35,A1:B5,2)';          tags = @('approximate', 'argcount'); note = 'omitted range_lookup defaults to TRUE' }
                @{ formula = '=VLOOKUP(5,A1:B5,2,TRUE)';      tags = @('approximate', 'boundary'); note = 'below every key' }
                @{ formula = '=VLOOKUP(99,A1:B5,2,TRUE)';     tags = @('approximate', 'boundary'); note = 'above every key' }
                @{ formula = '=VLOOKUP(30,A1:B5,2,TRUE)';     tags = @('approximate'); note = 'exact hit under approximate mode' }
                @{ formula = '=VLOOKUP(35,C1:D5,2,TRUE)';     tags = @('approximate', 'unsorted'); note = 'binary search over unsorted keys -- algorithm-defined' }
                @{ formula = '=VLOOKUP(10,C1:D5,2,FALSE)';    tags = @('exact', 'unsorted'); note = 'duplicate key: first wins' }
                @{ formula = '=VLOOKUP(30,A1:B5,3,FALSE)';    tags = @('error-input'); note = 'column index past the table' }
                @{ formula = '=VLOOKUP(30,A1:B5,0,FALSE)';    tags = @('error-input', 'boundary') }
                @{ formula = '=VLOOKUP(30,A1:B5,-1,FALSE)';   tags = @('error-input') }
                @{ formula = '=VLOOKUP(30,A1:B5,2.9,FALSE)';  tags = @('coercion') }
                @{ formula = '=VLOOKUP("banana",E1:F5,2,FALSE)'; tags = @('exact'); note = 'case-insensitive key match' }
                @{ formula = '=VLOOKUP("BANANA",E1:F5,2,FALSE)'; tags = @('exact') }
                @{ formula = '=VLOOKUP("a*",E1:F5,2,FALSE)';  tags = @('wildcard'); note = 'wildcards work in exact mode' }
                @{ formula = '=VLOOKUP("a?ple",E1:F5,2,FALSE)'; tags = @('wildcard') }
                @{ formula = '=VLOOKUP("a~*c",E1:F5,2,FALSE)'; tags = @('wildcard'); note = 'tilde escape matching the literal a*c key' }
                @{ formula = '=VLOOKUP(7,E1:F5,2,FALSE)';     tags = @('coercion'); note = 'number 7 against the text key "7"' }
                @{ formula = '=VLOOKUP("7",E1:F5,2,FALSE)';   tags = @('coercion') }
                @{ formula = '=VLOOKUP(G3,A1:B5,2,FALSE)';    tags = @('error-input') }
                @{ formula = '=VLOOKUP(30,A1:B5,2,"x")';      tags = @('error-input') }
                @{ formula = '=VLOOKUP(30,A1:A5,1,FALSE)';    tags = @('boundary'); note = 'single-column table' }
            )
        }

        @{
            name = 'HLOOKUP'
            doc  = 'Same semantics rotated. The fixture is a genuine horizontal table so the row index is exercised, not just aliased to VLOOKUP.'
            blocks = @(
                @{
                    label = 'horizontal table in A1:E2'
                    fixture = @(
                        @{ ref = 'A1'; value = 10 }
                        @{ ref = 'B1'; value = 20 }
                        @{ ref = 'C1'; value = 30 }
                        @{ ref = 'D1'; value = 40 }
                        @{ ref = 'E1'; value = 50 }
                        @{ ref = 'A2'; value = 'ten' }
                        @{ ref = 'B2'; value = 'twenty' }
                        @{ ref = 'C2'; value = 'thirty' }
                        @{ ref = 'D2'; value = 'forty' }
                        @{ ref = 'E2'; value = 'fifty' }
                        @{ ref = 'A3'; value = 'row3-a' }
                        @{ ref = 'C3'; formula = '=NA()' }
                    )
                    cases = @(
                        @{ formula = '=HLOOKUP(30,A1:E2,2,FALSE)';   tags = @('basic', 'exact') }
                        @{ formula = '=HLOOKUP(30,A1:E3,3,FALSE)';   tags = @('exact'); note = 'third row of the table is partly blank' }
                        @{ formula = '=HLOOKUP(35,A1:E2,2,FALSE)';   tags = @('exact') }
                        @{ formula = '=HLOOKUP(35,A1:E2,2,TRUE)';    tags = @('approximate') }
                        @{ formula = '=HLOOKUP(35,A1:E2,2)';         tags = @('approximate', 'argcount') }
                        @{ formula = '=HLOOKUP(5,A1:E2,2,TRUE)';     tags = @('approximate', 'boundary') }
                        @{ formula = '=HLOOKUP(99,A1:E2,2,TRUE)';    tags = @('approximate', 'boundary') }
                        @{ formula = '=HLOOKUP(30,A1:E2,1,FALSE)';   tags = @('exact') }
                        @{ formula = '=HLOOKUP(30,A1:E2,3,FALSE)';   tags = @('error-input'); note = 'row index past the table' }
                        @{ formula = '=HLOOKUP(30,A1:E2,0,FALSE)';   tags = @('error-input', 'boundary') }
                        @{ formula = '=HLOOKUP(C3,A1:E2,2,FALSE)';   tags = @('error-input') }
                        @{ formula = '=HLOOKUP(10,A1:A2,1,FALSE)';   tags = @('boundary'); note = 'single-column table' }
                    )
                }
            )
        }

        @{
            name = 'XLOOKUP'
            doc  = 'The modern replacement: an explicit not-found value, match modes -1/0/1/2 and search modes 1/-1/2/-2. On a build without native XLOOKUP the stored formula gains an _xlfn prefix, which the vector records.'
            cases = @(
                @{ formula = '=XLOOKUP(30,A1:A5,B1:B5)';           tags = @('basic') }
                @{ formula = '=XLOOKUP(35,A1:A5,B1:B5)';           tags = @('basic'); note = 'not found, no fallback given' }
                @{ formula = '=XLOOKUP(35,A1:A5,B1:B5,"none")';    tags = @('basic'); note = 'the explicit not-found value' }
                @{ formula = '=XLOOKUP(35,A1:A5,B1:B5,"none",-1)'; tags = @('approximate'); note = 'next smaller' }
                @{ formula = '=XLOOKUP(35,A1:A5,B1:B5,"none",1)';  tags = @('approximate'); note = 'next larger' }
                @{ formula = '=XLOOKUP("a*",E1:E5,F1:F5,"none",2)'; tags = @('wildcard'); note = 'match mode 2 enables wildcards' }
                @{ formula = '=XLOOKUP("a*",E1:E5,F1:F5,"none",0)'; tags = @('wildcard'); note = 'match mode 0 does not' }
                @{ formula = '=XLOOKUP(10,C1:C5,D1:D5,"none",0,1)'; tags = @('search-mode'); note = 'first to last over a duplicated key' }
                @{ formula = '=XLOOKUP(10,C1:C5,D1:D5,"none",0,-1)'; tags = @('search-mode'); note = 'last to first: the other duplicate' }
                @{ formula = '=XLOOKUP(99,A1:A5,B1:B5,"none",-1)'; tags = @('boundary') }
                @{ formula = '=XLOOKUP(5,A1:A5,B1:B5,"none",-1)';  tags = @('boundary'); note = 'below every key with next-smaller' }
                @{ formula = '=XLOOKUP(30,A1:A5,B1:B4)';           tags = @('error-input'); note = 'mismatched array lengths' }
                @{ formula = '=XLOOKUP(G3,A1:A5,B1:B5,"none")';    tags = @('error-input') }
                @{ formula = '=XLOOKUP("banana",E1:E5,F1:F5)';     tags = @('basic'); note = 'case-insensitive' }
                @{ formula = '=XLOOKUP(C5,C1:C5,D1:D5,"none")';    tags = @('blank'); note = 'looking up a blank' }
            )
        }

        @{
            name = 'INDEX'
            doc  = 'Zero means "the whole row/column", which in a single cell collapses via implicit intersection rather than erroring. That collapse is the part an engine gets wrong.'
            cases = @(
                @{ formula = '=INDEX(A1:B5,3,2)';   tags = @('basic') }
                @{ formula = '=INDEX(A1:B5,3,1)';   tags = @('basic') }
                @{ formula = '=INDEX(A1:A5,3)';     tags = @('basic'); note = 'single column, one coordinate' }
                @{ formula = '=INDEX(A1:E1,3)';     tags = @('basic'); note = 'single row, one coordinate' }
                @{ formula = '=INDEX(A1:B5,0,2)';   tags = @('boundary'); note = 'row 0 means the whole column' }
                @{ formula = '=INDEX(A1:B5,3,0)';   tags = @('boundary'); note = 'column 0 means the whole row' }
                @{ formula = '=INDEX(A1:B5,0,0)';   tags = @('boundary') }
                @{ formula = '=INDEX(A1:B5,6,1)';   tags = @('error-input'); note = 'past the last row' }
                @{ formula = '=INDEX(A1:B5,1,3)';   tags = @('error-input') }
                @{ formula = '=INDEX(A1:B5,-1,1)';  tags = @('error-input') }
                @{ formula = '=INDEX(A1:B5,2.9,1)'; tags = @('coercion') }
                @{ formula = '=INDEX(A1:B5,3)';     tags = @('argcount'); note = 'two-dimensional source, one coordinate' }
                @{ formula = '=SUM(INDEX(A1:B5,0,1))'; tags = @('boundary'); note = 'the whole-column form consumed as an array' }
                @{ formula = '=INDEX(C1:D5,5,2)';   tags = @('basic'); note = 'the row whose key is blank' }
                @{ formula = '=INDEX(A1:B5,G3,1)';  tags = @('error-input') }
            )
        }

        @{
            name = 'MATCH'
            doc  = 'Three match types with opposite sortedness contracts: 1 wants ascending, -1 wants descending, 0 wants nothing. Feeding the wrong order is where the interesting answers live.'
            cases = @(
                @{ formula = '=MATCH(30,A1:A5,0)';       tags = @('basic', 'exact') }
                @{ formula = '=MATCH(35,A1:A5,0)';       tags = @('exact') }
                @{ formula = '=MATCH(35,A1:A5,1)';       tags = @('approximate'); note = 'largest value not greater, ascending' }
                @{ formula = '=MATCH(35,A1:A5)';         tags = @('approximate', 'argcount'); note = 'omitted type defaults to 1' }
                @{ formula = '=MATCH(35,A1:A5,-1)';      tags = @('approximate'); note = 'type -1 over ascending data: contract violated' }
                @{ formula = '=MATCH(5,A1:A5,1)';        tags = @('boundary'); note = 'below every value' }
                @{ formula = '=MATCH(99,A1:A5,1)';       tags = @('boundary') }
                @{ formula = '=MATCH(10,C1:C5,0)';       tags = @('exact', 'unsorted'); note = 'first of two duplicates' }
                @{ formula = '=MATCH(35,C1:C5,1)';       tags = @('approximate', 'unsorted') }
                @{ formula = '=MATCH("banana",E1:E5,0)'; tags = @('exact'); note = 'case-insensitive' }
                @{ formula = '=MATCH("a*",E1:E5,0)';     tags = @('wildcard') }
                @{ formula = '=MATCH("a*",E1:E5,1)';     tags = @('wildcard'); note = 'wildcards under approximate match' }
                @{ formula = '=MATCH(7,E1:E5,0)';        tags = @('coercion') }
                @{ formula = '=MATCH(G3,A1:A5,0)';       tags = @('error-input') }
                @{ formula = '=MATCH(C5,C1:C5,0)';       tags = @('blank'); note = 'matching a blank' }
                @{ formula = '=MATCH(30,A1:B5,0)';       tags = @('error-input'); note = 'a two-dimensional lookup array' }
            )
        }

        @{
            name = 'SUMIF'
            doc  = 'Criteria are a small expression language in a string. The sum_range may be a different shape from the criteria range, in which case Excel resizes it silently from its top-left corner.'
            cases = @(
                @{ formula = '=SUMIF(A1:A5,">25")';            tags = @('basic'); note = 'no sum_range: sum the criteria range itself' }
                @{ formula = '=SUMIF(A1:A5,">25",H1:H5)';      tags = @('basic') }
                @{ formula = '=SUMIF(A1:A5,30,H1:H5)';         tags = @('basic'); note = 'a bare number as criteria' }
                @{ formula = '=SUMIF(A1:A5,"30",H1:H5)';       tags = @('coercion'); note = 'numeric text as criteria' }
                @{ formula = '=SUMIF(A1:A5,"=30",H1:H5)';      tags = @('basic') }
                @{ formula = '=SUMIF(A1:A5,"<>30",H1:H5)';     tags = @('basic') }
                @{ formula = '=SUMIF(A1:A5,">=30",H1:H5)';     tags = @('basic') }
                @{ formula = '=SUMIF(A1:A5,"<10",H1:H5)';      tags = @('boundary'); note = 'no matches' }
                @{ formula = '=SUMIF(A1:A5,">"&25,H1:H5)';     tags = @('basic'); note = 'criteria built by concatenation' }
                @{ formula = '=SUMIF(A1:A5,">"&A2,H1:H5)';     tags = @('basic'); note = 'criteria from a cell' }
                @{ formula = '=SUMIF(E1:E5,"a*",F1:F5)';       tags = @('wildcard') }
                @{ formula = '=SUMIF(E1:E5,"?anana",F1:F5)';   tags = @('wildcard') }
                @{ formula = '=SUMIF(E1:E5,"a~*c",F1:F5)';     tags = @('wildcard'); note = 'literal asterisk via the tilde escape' }
                @{ formula = '=SUMIF(E1:E5,"BANANA",F1:F5)';   tags = @('basic'); note = 'case-insensitive criteria' }
                @{ formula = '=SUMIF(A1:A5,">25",H1:H3)';      tags = @('boundary'); note = 'sum_range smaller than criteria range' }
                @{ formula = '=SUMIF(A1:A5,">25",H1)';         tags = @('boundary'); note = 'sum_range given as a single cell' }
                @{ formula = '=SUMIF(A1:A5,">25",H1:H5)+0';    tags = @('basic') }
                @{ formula = '=SUMIF(C1:C5,"",D1:D5)';         tags = @('blank'); note = 'empty-string criteria against a blank key' }
                @{ formula = '=SUMIF(C1:C5,"<>",D1:D5)';       tags = @('blank'); note = 'the not-blank idiom' }
                @{ formula = '=SUMIF(A1:A5,">25",A1:A5)';      tags = @('basic') }
            )
        }

        @{
            name = 'SUMIFS'
            doc  = 'sum_range comes first here and last in SUMIF. That argument-order inversion is Excel, not a slip, and it is worth a vector.'
            cases = @(
                @{ formula = '=SUMIFS(H1:H5,A1:A5,">25")';                  tags = @('basic') }
                @{ formula = '=SUMIFS(H1:H5,A1:A5,">15",A1:A5,"<45")';      tags = @('basic'); note = 'two criteria, same range' }
                @{ formula = '=SUMIFS(H1:H5,A1:A5,">25",E1:E5,"c*")';       tags = @('wildcard'); note = 'two criteria, different ranges' }
                @{ formula = '=SUMIFS(H1:H5,A1:A5,">99")';                  tags = @('boundary'); note = 'no matches' }
                @{ formula = '=SUMIFS(H1:H5,A1:A5,30)';                     tags = @('basic') }
                @{ formula = '=SUMIFS(H1:H5,A1:A5,">"&A1)';                 tags = @('basic') }
                @{ formula = '=SUMIFS(H1:H5,C1:C5,"")';                     tags = @('blank') }
                @{ formula = '=SUMIFS(H1:H5,A1:A5,">25",A1:A3,"<45")';      tags = @('error-input'); note = 'mismatched criteria range shapes' }
                @{ formula = '=SUMIFS(H1:H5,E1:E5,"BANANA")';               tags = @('basic') }
                @{ formula = '=SUMIFS(H1:H3,A1:A3,">15")';                  tags = @('basic'); note = 'the text "300" in H3 is not summable' }
            )
        }

        @{
            name = 'COUNTIF'
            cases = @(
                @{ formula = '=COUNTIF(A1:A5,">25")';       tags = @('basic') }
                @{ formula = '=COUNTIF(A1:A5,30)';          tags = @('basic') }
                @{ formula = '=COUNTIF(A1:A5,"30")';        tags = @('coercion') }
                @{ formula = '=COUNTIF(A1:A5,"<>30")';      tags = @('basic') }
                @{ formula = '=COUNTIF(E1:E5,"a*")';        tags = @('wildcard') }
                @{ formula = '=COUNTIF(E1:E5,"*a*")';       tags = @('wildcard') }
                @{ formula = '=COUNTIF(E1:E5,"?anana")';    tags = @('wildcard') }
                @{ formula = '=COUNTIF(E1:E5,"a~*c")';      tags = @('wildcard') }
                @{ formula = '=COUNTIF(E1:E5,7)';           tags = @('coercion'); note = 'number 7 against text key "7"' }
                @{ formula = '=COUNTIF(E1:E5,"7")';         tags = @('coercion') }
                @{ formula = '=COUNTIF(C1:C5,"")';          tags = @('blank'); note = 'does empty-string criteria count a blank cell' }
                @{ formula = '=COUNTIF(C1:C5,"<>")';        tags = @('blank') }
                @{ formula = '=COUNTIF(C1:C5,10)';          tags = @('basic'); note = 'duplicate keys' }
                @{ formula = '=COUNTIF(A1:A5,TRUE)';        tags = @('coercion') }
                @{ formula = '=COUNTIF(A1:A5,"*")';         tags = @('wildcard', 'boundary'); note = 'does * match numbers' }
            )
        }

        @{
            name = 'COUNTIFS'
            cases = @(
                @{ formula = '=COUNTIFS(A1:A5,">25")';                tags = @('basic') }
                @{ formula = '=COUNTIFS(A1:A5,">15",A1:A5,"<45")';    tags = @('basic') }
                @{ formula = '=COUNTIFS(A1:A5,">25",E1:E5,"c*")';     tags = @('wildcard') }
                @{ formula = '=COUNTIFS(A1:A5,">99")';                tags = @('boundary') }
                @{ formula = '=COUNTIFS(A1:A5,">15",A1:A3,"<45")';    tags = @('error-input'); note = 'mismatched shapes' }
                @{ formula = '=COUNTIFS(C1:C5,"")';                   tags = @('blank') }
                @{ formula = '=COUNTIFS(E1:E5,"*")';                  tags = @('wildcard') }
                @{ formula = '=COUNTIFS(A1:A5,10,E1:E5,"apple")';     tags = @('basic') }
            )
        }

        @{
            name = 'AVERAGEIF'
            cases = @(
                @{ formula = '=AVERAGEIF(A1:A5,">25")';           tags = @('basic') }
                @{ formula = '=AVERAGEIF(A1:A5,">25",H1:H5)';     tags = @('basic') }
                @{ formula = '=AVERAGEIF(A1:A5,">99")';           tags = @('boundary'); note = 'no matches: #DIV/0!, not 0' }
                @{ formula = '=AVERAGEIF(A1:A5,">25",H1:H3)';     tags = @('boundary'); note = 'shorter sum range' }
                @{ formula = '=AVERAGEIF(E1:E5,"a*",F1:F5)';      tags = @('wildcard') }
                @{ formula = '=AVERAGEIF(A1:A5,"<>30",H1:H5)';    tags = @('basic') }
                @{ formula = '=AVERAGEIF(C1:C5,"",D1:D5)';        tags = @('blank') }
                @{ formula = '=AVERAGEIF(A1:A5,">15",H1:H5)';     tags = @('basic'); note = 'H3 is text, so the denominator excludes it' }
                @{ formula = '=AVERAGEIF(A1:A5,30,H1:H5)';        tags = @('basic') }
            )
        }
    )
}
