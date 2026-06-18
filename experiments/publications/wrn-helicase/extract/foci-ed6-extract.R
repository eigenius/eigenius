# Vendored extraction: reshape the authors' ED Fig 6f/6h source panels
# (Apple-53BP1-trunc DSB foci per cell) from wrn_sourcedata_EDFig6_MOESM9.xlsx
# into a tidy long table for the file-backed DSB-foci SampleSet. Panels 6f
# (SW620 MSS, KM12 MSI) and 6h (ES2 MSS, OVK18 MSI) together span both MSI
# strata, so the MSI-selectivity of WRN-KO DSB induction is testable directly.
#
# Layout per sheet: row 1 title ("Number of Apple-53BP1-trunc foci per cell"),
# row 2 = cell-line label (every 4th col), row 3 = guide (sgCh2-2/sgWRN2/sgWRN3),
# rows 4+ = one foci count per cell (ragged, NA-filled).
#
# Run from data/slices/:  Rscript ../../extract/foci-ed6-extract.R
# Produces foci_53bp1_long.csv (cell_line, readout, guide, condition, value).
suppressMessages(library(readxl))
reshape_block <- function(file, sheet, readout) {
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
f <- "wrn_sourcedata_EDFig6_MOESM9.xlsx"
df <- rbind(
  reshape_block(f, "ED Fig 6f", "53BP1_foci"),
  reshape_block(f, "ED Fig 6h", "53BP1_foci")
)
df$condition <- ifelse(df$guide == "sgCh2-2", "control", "WRN_KO")
write.csv(df, "foci_53bp1_long.csv", row.names = FALSE)
cat(sprintf("wrote foci_53bp1_long.csv: %d cells\n", nrow(df)))
