| case | chunks | guest | session | user cycles | padded cycles | segments | GPU s (EST.) |
|---|---|---|---|---:|---:|---:|---:|
| light_only | 2 | accept | chunk | 195,961,166 | 232,783,872 | 111 | 255 |
| light_only | 2 | accept | chunk | 88,269,408 | 115,343,360 | 55 | 138 |
| light_only | 2 | accept | trie | 52,097,079 | 69,206,016 | 33 | 93 |
| light_only | 2 | accept | aggregate | 126,904,094 | 159,383,552 | 76 | 182 |
| **light_only** | 2 | accept | **all** | **463,231,747** | **576,716,800** | | **668 s = $0.46**; largest chunk 232,783,872 padded = 255 s; host peak RSS 1.28 GB |
| max_outputs | 2 | accept | chunk | 284,326,229 | 350,224,384 | 167 | 372 |
| max_outputs | 2 | accept | trie | 267,460,921 | 360,710,144 | 172 | 382 |
| max_outputs | 2 | accept | aggregate | 529,887,537 | 671,088,640 | 320 | 691 |
| **max_outputs** | 2 | accept | **all** | **1,081,674,687** | **1,382,023,168** | | **1,445 s = $1.00**; largest chunk 350,224,384 padded = 372 s; host peak RSS 1.54 GB |
| none_k8 | 2 | accept | chunk | 17,378,442,807 | 18,826,133,504 | 8,977 | 18,737 |
| none_k8 | 2 | accept | chunk | 89,656,048 | 117,440,512 | 56 | 141 |
| none_k8 | 2 | accept | trie | 52,122,855 | 69,206,016 | 33 | 93 |
| none_k8 | 2 | accept | aggregate | 135,642,742 | 168,820,736 | 81 | 192 |
| **none_k8** | 2 | accept | **all** | **17,655,864,452** | **19,181,600,768** | | **19,162 s = $13.25**; largest chunk 18,826,133,504 padded = 18,737 s; host peak RSS 1.27 GB |
| unsplit | 2 | accept | chunk | 1,428,669,021 | 1,629,487,104 | 777 | 1,643 |
| unsplit | 2 | accept | chunk | 88,301,120 | 115,474,432 | 56 | 139 |
| unsplit | 2 | accept | trie | 51,786,074 | 69,206,016 | 33 | 93 |
| unsplit | 2 | accept | aggregate | 135,124,126 | 167,804,928 | 81 | 191 |
| **unsplit** | 2 | accept | **all** | **1,703,880,341** | **1,981,972,480** | | **2,065 s = $1.43**; largest chunk 1,629,487,104 padded = 1,643 s; host peak RSS 1.27 GB |
| unsplit | 8 | accept | chunk | 1,381,295,386 | 1,574,961,152 | 751 | 1,589 |
| unsplit | 8 | accept | chunk | 40,959,976 | 60,817,408 | 29 | 84 |
| unsplit | 8 | accept | chunk | 40,950,886 | 60,817,408 | 29 | 84 |
| unsplit | 8 | accept | chunk | 40,941,796 | 60,817,408 | 29 | 84 |
| unsplit | 8 | accept | chunk | 40,932,706 | 60,817,408 | 29 | 84 |
| unsplit | 8 | accept | chunk | 40,923,616 | 60,817,408 | 29 | 84 |
| unsplit | 8 | accept | chunk | 40,914,526 | 60,817,408 | 29 | 84 |
| unsplit | 8 | accept | chunk | 40,900,342 | 60,817,408 | 29 | 84 |
| unsplit | 8 | accept | trie | 51,786,074 | 69,206,016 | 33 | 93 |
| unsplit | 8 | accept | aggregate | 135,169,588 | 167,772,160 | 80 | 191 |
| **unsplit** | 8 | accept | **all** | **1,854,774,896** | **2,237,661,184** | | **2,462 s = $1.70**; largest chunk 1,574,961,152 padded = 1,589 s; host peak RSS 1.66 GB |
| **mem_k1140** | 2 | reject/abort | **all** | **0** | **0** | | **0 s = $0.00**; largest chunk 0 padded = 24 s; host peak RSS 4.22 GB |
| **mem_k1140** | 8 | reject/abort | **all** | **0** | **0** | | **0 s = $0.00**; largest chunk 0 padded = 24 s; host peak RSS 8.77 GB |
| none_k24 | 2 | accept | chunk | 51,745,536,479 | 56,019,124,224 | 26,712 | 55,707 |
| none_k24 | 2 | accept | chunk | 92,331,496 | 121,634,816 | 58 | 145 |
| none_k24 | 2 | accept | trie | 52,168,469 | 69,206,016 | 33 | 93 |
| none_k24 | 2 | accept | aggregate | 135,628,963 | 168,820,736 | 81 | 192 |
| **none_k24** | 2 | accept | **all** | **52,025,665,407** | **56,378,785,792** | | **56,136 s = $38.83**; largest chunk 56,019,124,224 padded = 55,707 s; host peak RSS 1.27 GB |
| all_k8 | 2 | accept | chunk | 7,032,152,244 | 7,761,559,552 | 3,701 | 7,739 |
| all_k8 | 2 | accept | chunk | 89,656,066 | 117,440,512 | 56 | 141 |
| all_k8 | 2 | accept | trie | 52,125,761 | 69,206,016 | 33 | 93 |
| all_k8 | 2 | accept | aggregate | 135,878,494 | 168,820,736 | 81 | 192 |
| **all_k8** | 2 | accept | **all** | **7,309,812,565** | **8,117,026,816** | | **8,163 s = $5.65**; largest chunk 7,761,559,552 padded = 7,739 s; host peak RSS 1.27 GB |
| none_small_k8 | 2 | accept | chunk | 107,186,020 | 114,294,784 | 55 | 137 |
| none_small_k8 | 2 | accept | chunk | 107,430,343 | 114,294,784 | 55 | 137 |
| none_small_k8 | 2 | accept | trie | 90,624 | 262,144 | 1 | 24 |
| none_small_k8 | 2 | accept | aggregate | 2,665,823 | 3,145,728 | 2 | 27 |
| **none_small_k8** | 2 | accept | **all** | **217,372,810** | **231,997,440** | | **326 s = $0.23**; largest chunk 114,294,784 padded = 137 s; host peak RSS 1.18 GB |
| unsplit_none | 8 | accept | chunk | 4,416,938,017 | 4,796,186,624 | 2,287 | 4,791 |
| unsplit_none | 8 | accept | chunk | 40,959,928 | 60,817,408 | 29 | 84 |
| unsplit_none | 8 | accept | chunk | 40,950,838 | 60,817,408 | 29 | 84 |
| unsplit_none | 8 | accept | chunk | 40,941,748 | 60,817,408 | 29 | 84 |
| unsplit_none | 8 | accept | chunk | 40,932,658 | 60,817,408 | 29 | 84 |
| unsplit_none | 8 | accept | chunk | 40,923,568 | 60,817,408 | 29 | 84 |
| unsplit_none | 8 | accept | chunk | 40,914,478 | 60,817,408 | 29 | 84 |
| unsplit_none | 8 | accept | chunk | 40,900,294 | 60,817,408 | 29 | 84 |
| unsplit_none | 8 | accept | trie | 51,786,141 | 69,206,016 | 33 | 93 |
| unsplit_none | 8 | accept | aggregate | 135,158,425 | 167,772,160 | 80 | 191 |
| **unsplit_none** | 8 | accept | **all** | **4,890,406,095** | **5,458,886,656** | | **5,664 s = $3.92**; largest chunk 4,796,186,624 padded = 4,791 s; host peak RSS 1.66 GB |
| mem_k1140 | 32 | accept | chunk | 3,845,653,695 | 5,117,575,168 | 2,441 | 5,111 |
| mem_k1140 | 32 | accept | chunk | 3,762,683,164 | 5,026,873,344 | 2,397 | 5,020 |
| mem_k1140 | 32 | accept | chunk | 3,762,682,552 | 5,026,873,344 | 2,397 | 5,020 |
| mem_k1140 | 32 | accept | chunk | 3,762,681,940 | 5,026,873,344 | 2,397 | 5,020 |
| mem_k1140 | 32 | accept | chunk | 3,762,681,328 | 5,026,873,344 | 2,397 | 5,020 |
| mem_k1140 | 32 | accept | chunk | 2,242,850,715 | 3,008,364,544 | 1,435 | 3,014 |
| mem_k1140 | 32 | accept | chunk | 71,657,377 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,654,737 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,652,097 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,649,457 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,646,817 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,644,177 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,641,537 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,638,897 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,636,257 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,633,617 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,630,977 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,628,337 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,625,697 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,623,057 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,620,417 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,617,777 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,615,137 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,612,497 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,609,857 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,607,217 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,604,577 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,601,937 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,599,297 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,596,657 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,594,017 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | chunk | 71,540,849 | 125,829,120 | 60 | 149 |
| mem_k1140 | 32 | accept | trie | 52,539,643 | 69,238,784 | 34 | 93 |
| mem_k1140 | 32 | accept | aggregate | 136,215,010 | 168,820,736 | 81 | 192 |
| **mem_k1140** | 32 | accept | **all** | **23,190,171,321** | **31,743,049,728** | | **32,361 s = $22.38**; largest chunk 5,117,575,168 padded = 5,111 s; host peak RSS 15.07 GB |
| mem_k1140 | 128 | accept | chunk | 1,073,960,299 | 1,438,646,272 | 686 | 1,454 |
| mem_k1140 | 128 | accept | chunk | 991,019,948 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 990,990,170 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 990,989,974 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 990,989,768 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 991,019,336 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 990,989,558 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 990,989,362 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 990,989,156 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 991,018,724 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 990,988,946 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 990,988,750 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 990,988,544 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 991,018,112 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 990,988,334 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 990,988,138 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 990,987,932 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 991,017,500 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 990,987,722 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 990,987,526 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 990,987,320 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 991,016,888 | 1,346,371,584 | 642 | 1,362 |
| mem_k1140 | 128 | accept | chunk | 393,907,531 | 553,648,128 | 264 | 574 |
| mem_k1140 | 128 | accept | chunk | 68,238,711 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,238,051 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,237,391 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,236,731 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,236,071 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,235,411 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,234,751 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,234,091 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,233,431 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,232,771 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,232,111 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,231,451 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,230,791 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,230,131 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,229,471 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,228,811 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,228,151 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,227,491 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,226,831 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,226,171 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,225,511 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,224,851 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,224,191 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,223,531 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,222,871 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,222,211 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,221,551 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,220,891 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,220,231 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,219,571 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,218,911 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,218,251 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,217,591 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,216,931 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,216,271 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,215,611 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,214,951 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,214,291 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,213,631 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,212,971 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,212,311 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,211,651 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,210,991 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,210,331 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,209,671 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,209,011 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,208,351 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,207,691 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,207,031 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,206,371 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,205,711 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,205,051 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,204,391 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,203,731 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,203,071 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,202,411 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,201,751 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,201,091 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,200,431 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,199,771 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,199,111 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,198,451 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,197,791 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,197,131 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,196,471 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,195,811 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,195,151 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,194,491 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,193,831 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,193,171 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,192,511 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,191,851 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,191,191 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,190,531 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,189,871 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,189,211 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,188,551 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,187,891 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,187,231 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,186,571 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,185,911 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,185,251 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,184,591 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,183,931 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,183,271 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,182,611 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,181,951 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,181,291 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,180,631 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,179,971 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,179,311 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,178,651 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,177,991 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,177,331 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,176,671 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,176,011 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,175,351 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,174,691 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,174,031 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,173,371 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,172,711 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,172,051 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,171,391 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,170,731 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | chunk | 68,129,486 | 121,634,816 | 58 | 145 |
| mem_k1140 | 128 | accept | trie | 52,539,643 | 69,238,784 | 34 | 93 |
| mem_k1140 | 128 | accept | aggregate | 136,932,314 | 169,869,312 | 81 | 193 |
| **mem_k1140** | 128 | accept | **all** | **29,629,701,965** | **43,276,861,440** | | **46,107 s = $31.89**; largest chunk 1,438,646,272 padded = 1,454 s; host peak RSS 11.39 GB |

- CASE light_only: height=126 bytes=999583 weight=3998224 txs=2 inputs=24374 outputs=3 core=accept core_submit_s=0.15
- CASE max_outputs: height=126 bytes=999723 weight=3998784 txs=2 inputs=1 outputs=111051 core=accept core_submit_s=0.38
- CASE none_k8: height=126 bytes=999583 weight=3998224 txs=2 inputs=24374 outputs=3 core=accept core_submit_s=0.60
- CASE unsplit: height=126 bytes=999586 weight=3998236 txs=2 inputs=24212 outputs=3 core=accept core_submit_s=0.78
- CASE mem_k1140: height=126 bytes=999583 weight=None txs=2 inputs=24374 outputs=3 core=duplicate-invalid core_submit_s=0.09
- CASE none_k24: height=126 bytes=999583 weight=3998224 txs=2 inputs=24374 outputs=3 core=accept core_submit_s=0.83
- CASE all_k8: height=126 bytes=999583 weight=3998224 txs=2 inputs=24374 outputs=3 core=accept core_submit_s=0.52
- CASE none_small_k8: height=126 bytes=575 weight=2192 txs=2 inputs=8 outputs=3 core=accept core_submit_s=0.02
- CASE unsplit_none: height=126 bytes=999586 weight=3998236 txs=2 inputs=24212 outputs=3 core=accept core_submit_s=0.83
- mem_k1140 chunks=2: chunk execution failed: Guest panicked: Out of memory! You have been using the default bump allocator which does not reclaim memory. Enable the `heap-embedded-alloc` feature to reclaim memory. This will result in extra cycle cost. | thread 'main' (1241413) panicked at host/src/lz.rs:695:5: | a scoped thread panicked
- mem_k1140 chunks=8: chunk execution failed: Guest panicked: Out of memory! You have been using the default bump allocator which does not reclaim memory. Enable the `heap-embedded-alloc` feature to reclaim memory. This will result in extra cycle cost. | thread 'main' (1245776) panicked at host/src/lz.rs:695:5: | a scoped thread panicked
