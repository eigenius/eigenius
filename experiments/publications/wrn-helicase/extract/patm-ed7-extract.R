# Vendored extraction: reshape the authors' ED Fig 7b/7d source panels (nuclear
# phospho-ATM(S1981) foci per cell) from wrn_sourcedata_EDFig7_MOESM10.xlsx into
# a tidy long table for the file-backed pATM-foci SampleSet. Panel 7b (colon:
# SW620 MSS, KM12 MSI, SW48 MSI) and 7d (ovarian: ES2 MSS, OVK18 MSI) together
# span both MSI strata. pATM(S1981) autophosphorylation foci report activation of
# the apical ATM DSB-response kinase — the signaling step the paper uses to
# bridge DSBs → p53 / anti-proliferative signaling ("DSB responses known to
# activate p53", ED 7). This closes the DDR-signaling leg
# (onco:ActivatesDSBResponse), previously absent from the chain.
#
# Layout per sheet: row 2 = cell-line label, row 3 = guide (sgCh2-2/sgWRN2/sgWRN3)
# marking the FIRST sub-column of a 3-column block [intensity, pan-nuclear?,
# #foci]; row 4 = sub-headers; rows 5+ = values. The published quantity is the
# #foci sub-column (guide column + 2); pan-nuclear cells (~3-6%, uncountable
# discrete foci) carry a blank #foci and are dropped. Any right-hand "number of
# cells" block is cut before parsing.
#
# Run from data/slices/:  Rscript ../../extract/patm-ed7-extract.R
# Produces patm_foci_long.csv (cell_line, readout, guide, condition, value).
suppressMessages(library(readxl))
f <- "wrn_sourcedata_EDFig7_MOESM10.xlsx"
reshape_foci <- function(sheet) {
  raw <- as.data.frame(suppressMessages(
    read_excel(f, sheet = sheet, col_names = FALSE, col_types = "text")))
  hdr <- which(apply(raw, 2, function(c) any(grepl("number of cells", c, ignore.case = TRUE))))
  sub <- raw[, seq_len(if (length(hdr)) min(hdr) - 1 else ncol(raw)), drop = FALSE]
  rcell <- as.character(unlist(sub[2, ])); rguide <- as.character(unlist(sub[3, ]))
  cl <- rcell; for (i in seq_along(cl)) if (i > 1 && is.na(cl[i])) cl[i] <- cl[i - 1]
  recs <- list()
  for (j in seq_along(rguide)) {
    g <- rguide[j]; if (is.na(g) || !g %in% c("sgCh2-2", "sgWRN2", "sgWRN3")) next
    # #foci is the third sub-column of the guide block (intensity, pan-nuclear, #foci).
    v <- suppressWarnings(as.numeric(sub[5:nrow(sub), j + 2])); v <- v[!is.na(v)]
    if (length(v)) recs[[length(recs) + 1]] <- data.frame(
      cell_line = cl[j], readout = "pATM_foci", guide = g, value = v)
  }
  do.call(rbind, recs)
}
df <- rbind(reshape_foci("ED Fig 7b"), reshape_foci("ED Fig 7d"))
df$condition <- ifelse(df$guide == "sgCh2-2", "control", "WRN_KO")
write.csv(df, "patm_foci_long.csv", row.names = FALSE)
cat(sprintf("wrote patm_foci_long.csv: %d cells\n", nrow(df)))
