# Vendored extraction: reshape the authors' ED Fig 5 source-data workbook
# (wrn_sourcedata_EDFig5_MOESM8.xlsx, panels 5b/5d/5f) from its ragged
# stacked-block layout into a tidy long table for the file-backed IF SampleSet.
#
# Layout per sheet: a "<readout> staining intensity" block (left) and a
# "number of cells counted" block (right, ignored). Within the intensity block,
# row 2 = cell-line label (every 4th col), row 3 = guide (sgCh2-2 / sgWRN2 /
# sgWRN3), rows 4+ = one staining-intensity value per cell (ragged, NA-filled).
#
# Run from data/slices/:  Rscript ../../extract/if-ed5-extract.R
# Produces if_ed5_long.csv (cell_line, readout, guide, condition, value).
suppressMessages(library(readxl))
reshape_if <- function(file, sheet, readout) {
  raw <- as.data.frame(suppressMessages(read_excel(file, sheet = sheet, col_names = FALSE, col_types = "text")))
  hdr <- which(apply(raw, 2, function(c) any(grepl("number of cells", c, ignore.case = TRUE))))
  sub <- raw[, seq_len(if (length(hdr)) min(hdr) - 1 else ncol(raw)), drop = FALSE]
  rcell <- as.character(unlist(sub[2, ])); rguide <- as.character(unlist(sub[3, ]))
  cl <- rcell; for (i in seq_along(cl)) if (i > 1 && is.na(cl[i])) cl[i] <- cl[i - 1]
  recs <- list()
  for (j in seq_along(rguide)) {
    g <- rguide[j]; if (is.na(g) || !g %in% c("sgCh2-2", "sgWRN2", "sgWRN3")) next
    v <- suppressWarnings(as.numeric(sub[4:nrow(sub), j])); v <- v[!is.na(v)]
    if (length(v)) recs[[length(recs) + 1]] <- data.frame(cell_line = cl[j], readout = readout, guide = g, value = v)
  }
  do.call(rbind, recs)
}
f <- "wrn_sourcedata_EDFig5_MOESM8.xlsx"
df <- rbind(
  reshape_if(f, "ED Fig 5b", "p_p53"),
  reshape_if(f, "ED Fig 5d", "p21"),
  reshape_if(f, "ED Fig 5f", "p21")
)
df$condition <- ifelse(df$guide == "sgCh2-2", "control", "WRN_KO")
write.csv(df, "if_ed5_long.csv", row.names = FALSE)
cat(sprintf("wrote if_ed5_long.csv: %d rows, %d cells\n", nrow(df), nrow(df)))
