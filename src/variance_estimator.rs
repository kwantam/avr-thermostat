//! Approximate the variance of a signal I by computing
//! EWMA(I^2) - EWMA(I)^2

/// Leaky bucket leak rate. Higher gives slower time constant and more precision.
#[derive(Default, Clone, Copy)]
#[repr(u8)]
pub enum VarianceEstimatorLeak {
    Leak16 = 4,
    Leak32 = 5,
    Leak64 = 6,
    Leak128 = 7,
    #[default]
    Leak256 = 8,
}

/// An estimator of variance computed via EWMA(I^2) - EWMA(I)^2
#[derive(Default, Clone, Copy)]
pub struct VarianceEstimator {
    leak: VarianceEstimatorLeak,
    msq: u32,
    mean: u32,
}

impl VarianceEstimator {
    /// Initialize a new variance estimator.
    pub fn new(leak: VarianceEstimatorLeak) -> Self {
        Self {
            leak,
            ..Default::default()
        }
    }

    /// Update the current variance estimate using the supplied value.
    ///
    /// NOTE: `val` is clipped to <1024, since this is the max output of the ADC.
    pub fn put(&mut self, val: u32) {
        let val = core::cmp::min(val, 1023);
        let leak = self.leak as u8;

        self.mean -= self.mean >> leak;
        self.mean += val;

        // at most 20 bits
        let vsq = val * val;
        self.msq -= self.msq >> leak;
        self.msq += vsq;
    }

    /// Get the current variance estimate without updating
    pub fn get_variance(&self) -> u32 {
        // self.msq is roughly mean(v^2) * 2^leak.
        // self.mean is roughly mean(v) * 2^leak.
        //
        // We could compute (msq >> leak) - (mean >> leak)^2, but
        // if we want the most precision we could instead compute
        //
        // msq - (mean^2 >> leak)
        //   = mean(v^2) * 2^leak - (mean(v)^2 * 2^(2*leak) / 2^leak)
        //   = mean(v^2) * 2^leak - mean(v)^2 * 2^leak
        //   = (mean(v^2) - mean(v)^2) * 2^leak
        //
        // One issue is that we may run out of bits doing this.
        // Values are 10 bits. When leak is 8, we have that msq
        // is bounded by 2^28 and mean is bounded by 2^18. That
        // means mean^2 does not fit in a 32-bit register.
        //
        // To handle this, we start by dividing `mean` by 4. This
        // gives us an upper bound of 2^16, whose square fits into
        // a register. After squaring, we normalize a bit more.
        //
        // For leak L, we need to divide the squared value by 2^L.
        // Dividing by two before squaring is equivalent to dividing
        // the squared value by 2^4, so we just need to divide by the
        // remaining amount, i.e., 2^(L - 4).

        let square_of_mean = match self.leak {
            VarianceEstimatorLeak::Leak256 => {
                // mean is at most 10 + 8 = 18 bits
                // at most 16 bits
                let norm_mean = self.mean >> 2;
                // at most 32 bits
                let square_of_mean = norm_mean * norm_mean;
                // mean(v)^2 * 2^leak
                square_of_mean >> 4
            }
            VarianceEstimatorLeak::Leak128 => {
                // mean is at most 10 + 7 = 17 bits
                // at most 16 bits
                let norm_mean = self.mean >> 1;
                // at most 32 bits
                let square_of_mean = norm_mean * norm_mean;
                // mean(v)^2 * 2^leak
                square_of_mean >> 5
            }
            VarianceEstimatorLeak::Leak64
            | VarianceEstimatorLeak::Leak32
            | VarianceEstimatorLeak::Leak16 => {
                // mean is at most 10 + leak <= 16 bits
                // at most 32 bits
                let square_of_mean = self.mean * self.mean;
                // mean(v)^2 * 2^leak
                square_of_mean >> (self.leak as u8)
            }
        };
        self.msq - square_of_mean
    }

    /// Get EWMA(I) without updating
    pub fn get_mean(&self) -> u32 {
        self.mean
    }

    /// Get EWMA(I^2) without updating
    pub fn get_msq(&self) -> u32 {
        self.msq
    }
}
