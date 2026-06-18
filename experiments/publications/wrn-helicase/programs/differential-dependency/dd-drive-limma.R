suppressMessages({library(data.table); library(limma)})
dt  <- fread("drive_D2_DRIVE_gene_dep_scores.csv")
g   <- dt[[1]]; M <- as.matrix(dt[, -1]); rownames(M) <- g   # genes x cell-lines
s1  <- fread("wrn_supplementary_table_1.csv")
msi <- s1$CCLE_MSI[match(colnames(M), s1$CCLE_ID)]            # DRIVE cols ARE CCLE_IDs
keep <- !is.na(msi) & msi %in% c("MSI","MSS")
M <- M[, keep]; grp <- factor(msi[keep], levels=c("MSS","MSI"))
cat(sprintf("cell lines: %d (MSI %d / MSS %d), genes: %d\n", ncol(M), sum(grp=="MSI"), sum(grp=="MSS"), nrow(M)))
# DRIVE is already genes(features) x cell-lines(samples); drop near-empty genes
M <- M[rowSums(!is.na(M)) >= 3, ]
design <- model.matrix(~ grp)            # coef 2 = MSI vs MSS
fit <- eBayes(lmFit(M, design))
tt  <- topTable(fit, coef=2, number=Inf, sort.by="none")
tt$gene <- rownames(tt)
# MSI-preferential = more dependent in MSI = lower DRIVE score = logFC < 0; rank by P
neg <- tt[tt$logFC < 0, ]; neg <- neg[order(neg$P.Value), ]
cat("Top 8 MSI-preferential by limma moderated-t:\n")
print(head(neg[,c("gene","logFC","t","P.Value","adj.P.Val")],8))
w <- which(grepl("^WRN \\(", neg$gene))[1]
cat(sprintf("\nWRN rank among MSI-preferential: %d of %d\n", w, nrow(neg)))
cat(sprintf("WRN: logFC=%.3f  modt=%.2f  P=%.3g  adj.P(Q)=%.3g\n",
            neg$logFC[w], neg$t[w], neg$P.Value[w], neg$adj.P.Val[w]))
