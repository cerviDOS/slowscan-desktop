import sys
import subprocess
import pandas as pd

#teensy_output = subprocess.Popen(['tail', '-n 1000', sys.argv[1]], \
#    stdout=subprocess.PIPE)

#data = pd.read_csv(teensy_output.stdout, sep=',',header=None)
data = pd.read_csv(sys.argv[1], sep=',',header=None)
data = pd.DataFrame(data)

import matplotlib.pyplot as plt
import numpy as np

I = data[0]
#Q = data[1]


plt.plot(I, 'r')
#plt.plot(Q, 'b')
plt.show()



