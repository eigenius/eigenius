# Vendored extraction: reshape the authors' ED Fig 6c source panel (nuclear
# γH2AX staining intensity per cell) from wrn_sourcedata_EDFig6_MOESM9.xlsx into
# a tidy long table for the file-backed γH2AX-intensity SampleSet. ED 6c spans
# ES2 (MSS ovarian) and OVK18 (MSI ovarian), so the MSI-selectivity of WRN-KO
# γH2AX induction is testable directly — and the panel carries the paper's exact
# published statistic (contrast-of-LSM P < 2×10⁻¹⁶; mean log10 fold-change 0.147
# OVK18 / 0.055 ES2). γH2AX is the canonical DSB lesion marker (named before
# 53BP1 in the text); this closes its leg of CausesDSBs.
#
# Layout: row 2 title ("γH2AX staining intensity" | "number of cells counted"),
# row 3 = cell-line label, row 4 = guide (sgCh2-2/sgWRN2/sgWRN3), rows 5+ = one
# intensity value per cell. The "number of cells" block (right of the intensity
# block, also guide-split) is cut off before parsing.
#
# Run from data/slices/:  Rscript ../../extract/gh2ax-ed6c-extract.R
# Produces gh2ax_intensity_long.csv (cell_line, readout, guide, condition, value).
suppressMessages(library(readxl))
f <- "wrn_sourcedata_EDFig6_MOESM9.xlsx"
raw <- as.data.frame(suppressMessages(
  read_excel(f, sheet = "ED Fig 6c", col_names = FALSE, col_types = "text")))
# Cut the intensity block off before the "number of cells counted" block.
hdr <- which(apply(raw, 2, function(c) any(grepl("number of cells", c, ignore.case = TRUE))))
sub <- raw[, seq_len(if (length(hdr)) min(hdr) - 1 else ncol(raw)), drop = FALSE]
rcell <- as.character(unlist(sub[2, ])); rguide <- as.character(unlist(sub[3, ]))
cl <- rcell; for (i in seq_along(cl)) if (i > 1 && is.na(cl[i])) cl[i] <- cl[i - 1]
recs <- list()
for (j in seq_along(rguide)) {
  g <- rguide[j]; if (is.na(g) || !g %in% c("sgCh2-2", "sgWRN2", "sgWRN3")) next
  v <- suppressWarnings(as.numeric(sub[4:nrow(sub), j])); v <- v[!is.na(v)]
  if (length(v)) recs[[length(recs) + 1]] <- data.frame(
    cell_line = cl[j], readout = "gH2AX_intensity", guide = g, value = v)
}
df <- do.call(rbind, recs)
df$condition <- ifelse(df$guide == "sgCh2-2", "control", "WRN_KO")
write.csv(df, "gh2ax_intensity_long.csv", row.names = FALSE)
cat(sprintf("wrote gh2ax_intensity_long.csv: %d cells\n", nrow(df)))
