# Vendored extraction: reshape the authors' ED Fig 6a (colon) + 6d (ovarian)
# source panels (nuclear γH2AX foci per cell) from
# wrn_sourcedata_EDFig6_MOESM9.xlsx into a tidy long table for the file-backed
# γH2AX-foci SampleSet. Panels span both MSI strata (colon SW620 MSS / KM12,SW48
# MSI; ovarian ES2 MSS / OVK18 MSI).
#
# γH2AX is the *diffuse* DSB marker: at high damage the nucleus saturates and the
# discrete foci become uncountable — recorded as "Pan-nuclear? = YES" with a blank
# #foci. Those saturated cells are the MOST-damaged ones (disproportionately MSI
# WRN-KO: pan-nuclear fraction KM12 13%->50%, SW48 1%->21% on WRN loss), so they
# must be COUNTED, not dropped. We assign pan-nuclear cells a saturation ceiling =
# the maximum countable foci value over the panel (a conservative censoring of an
# uncountable value). Dropping them instead inverts the result (a spurious
# decrease) — which is why ED 6c intensity is the paper's primary γH2AX statistic,
# but the foci panel (handled this way) is a genuine, strongly MSI-selective
# readout (interaction +7.3, fold-change MSI x3.4 vs MSS x1.0).
#
# Layout per sheet: row 1 = cell-line label, row 2 = guide (sgCh2-2/sgWRN2/sgWRN3)
# marking the FIRST sub-column of a 3-column block [intensity, pan-nuclear?,
# #foci]; row 3 = sub-headers; rows 4+ = values. Any right-hand "number of cells"
# block is cut before parsing.
#
# Run from data/slices/:  Rscript ../../extract/gh2ax-ed6ad-extract.R
# Produces gh2ax_foci_long.csv (cell_line, readout, guide, condition, value).
suppressMessages(library(readxl))
f <- "wrn_sourcedata_EDFig6_MOESM9.xlsx"
collect <- function(sheet) {
  raw <- as.data.frame(suppressMessages(
    read_excel(f, sheet = sheet, col_names = FALSE, col_types = "text")))
  hdr <- which(apply(raw, 2, function(c) any(grepl("number of cells", c, ignore.case = TRUE))))
  sub <- raw[, seq_len(if (length(hdr)) min(hdr) - 1 else ncol(raw)), drop = FALSE]
  rcell <- as.character(unlist(sub[1, ])); rguide <- as.character(unlist(sub[2, ]))
  cl <- rcell; for (i in seq_along(cl)) if (i > 1 && is.na(cl[i])) cl[i] <- cl[i - 1]
  recs <- list()
  for (j in seq_along(rguide)) {
    g <- rguide[j]; if (is.na(g) || !g %in% c("sgCh2-2", "sgWRN2", "sgWRN3")) next
    pan  <- toupper(trimws(as.character(sub[4:nrow(sub), j + 1])))
    foci <- suppressWarnings(as.numeric(sub[4:nrow(sub), j + 2]))
    keep <- pan %in% c("YES", "NO")
    if (any(keep)) recs[[length(recs) + 1]] <- data.frame(
      cell_line = cl[j], guide = g, pan = pan[keep], foci = foci[keep])
  }
  do.call(rbind, recs)
}
df <- rbind(collect("ED Fig 6a"), collect("ED Fig 6d"))
# Saturation ceiling: pan-nuclear (uncountable) cells get the max countable foci.
ceil <- max(df$foci, na.rm = TRUE)
df$value <- ifelse(df$pan == "YES", ceil, df$foci)
df <- df[!is.na(df$value), ]
df$readout <- "gH2AX_foci"
df$condition <- ifelse(df$guide == "sgCh2-2", "control", "WRN_KO")
df <- df[, c("cell_line", "readout", "guide", "condition", "value")]
write.csv(df, "gh2ax_foci_long.csv", row.names = FALSE)
cat(sprintf("wrote gh2ax_foci_long.csv: %d cells (saturation ceiling = %g)\n", nrow(df), ceil))
