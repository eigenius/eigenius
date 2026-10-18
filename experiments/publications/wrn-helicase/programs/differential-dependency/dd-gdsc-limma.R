# D-DIFF robustness (Achilles, GDSC PCR-MSI labels): WRN is STILL the top
# preferential dependency when MSI is called by the orthogonal GDSC PCR panel
# (MSI-H vs MSS/MSI-L) rather than the NGS CCLE_MSI calls used in dd_achilles.
suppressMessages({library(data.table); library(limma)})
dt  <- fread("achilles_18Q4_gene_effect.csv")
ids <- dt[[1]]; M <- as.matrix(dt[, -1]); rownames(M) <- ids
si  <- fread("achilles_18Q4_sample_info.csv")
s1  <- fread("wrn_supplementary_table_1.csv")
ccle <- si$CCLE_name[match(ids, si$DepMap_ID)]
g    <- s1$GDSC_MSI[match(ccle, s1$CCLE_ID)]
keep <- !is.na(g) & g %in% c("MSI-H","MSS/MSI-L")
M <- M[keep,]; grp <- factor(g[keep], levels=c("MSS/MSI-L","MSI-H"))
cat(sprintf("cell lines: %d (MSI-H %d / MSS/MSI-L %d), genes: %d\n", nrow(M), sum(grp=="MSI-H"), sum(grp=="MSS/MSI-L"), ncol(M)))
fit <- eBayes(lmFit(t(M), model.matrix(~ grp)))   # coef 2 = MSI-H vs MSS/MSI-L
tt  <- topTable(fit, coef=2, number=Inf, sort.by="none"); tt$gene <- rownames(tt)
neg <- tt[tt$logFC < 0, ]; neg <- neg[order(neg$P.Value), ]
w <- which(grepl("^WRN \\(", neg$gene))[1]
cat(sprintf("WRN rank: %d of %d; logFC=%.3f modt=%.2f Q=%.3g\n",
            w, nrow(neg), neg$logFC[w], neg$t[w], neg$adj.P.Val[w]))
